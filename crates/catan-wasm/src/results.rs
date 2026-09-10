use super::*;

pub(super) struct ScoreFrame {
    pub turn: u32,
    pub points: [u8; MAX_PLAYERS],
}

// Keep the last state of every turn, including losses of road/army awards.
// Replay uses apply_notify too, so reconnecting restores the same history.
pub(super) fn record(sess: &mut Session) {
    let g = &sess.game;
    let turn = if g.is_setup() { 0 } else { g.turn };
    let points = std::array::from_fn(|p| if p < g.n() { g.actual_vp(p as u8) } else { 0 });
    let frame = ScoreFrame { turn, points };
    if sess.scores.last().is_some_and(|last| last.turn == turn) {
        *sess.scores.last_mut().unwrap() = frame;
    } else {
        sess.scores.push(frame);
    }
}

/// Hidden victory cards are revealed only after the game ends.
#[no_mangle]
pub extern "C" fn result_json() -> *const u8 {
    SESSION.with(|s| {
        let b = s.borrow();
        let Some(sess) = b.as_ref().filter(|s| s.game.is_over()) else { return put_out("null".into()); };
        let mut j = Json::new();
        j.obj_start();
        j.key("timeline"); j.arr_start();
        for frame in &sess.scores {
            j.obj_start(); j.key("turn"); j.num(frame.turn as f32);
            j.key("points"); j.arr_start();
            for p in 0..sess.game.n() { j.num(frame.points[p] as f32); }
            j.arr_end(); j.obj_end();
        }
        j.arr_end();
        j.key("rollCounts"); j.arr_start();
        for count in &sess.rolls[2..] { j.num(*count as f32); }
        j.arr_end(); j.obj_end(); put_out(j.finish())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn output() -> String { OUT.with(|o| String::from_utf8(o.borrow().clone()).unwrap()) }
    #[test]
    fn history_is_private_until_end_and_tracks_gains_losses_and_new_games() {
        game_new(42,2,3,4,4,1,0,1);
        result_json(); assert_eq!(output(), "null");
        SESSION.with(|s| {
            let mut b=s.borrow_mut(); let sess=b.as_mut().unwrap();
            sess.game.setup_index=8; sess.game.turn=1;
            sess.game.players[0].dev[DevCard::VictoryPoint.idx()]=1;
            sess.game.longest_road_owner=Some(0);
            record(sess); assert_eq!(sess.scores.last().unwrap().points[0],3);
            sess.game.turn=2; sess.game.longest_road_owner=Some(1);
            record(sess); assert_eq!(sess.scores.last().unwrap().points[0],1);
            assert_eq!(sess.scores[1].points[0],3);
            sess.game.winner=Some(1); sess.game.prompt=Prompt::GameOver;
        });
        result_json(); assert!(output().contains("\"timeline\""));
        assert!(output().contains("\"points\":[1,2,0,0]"));
        game_new(42,2,3,4,3,1,0,1);
        SESSION.with(|s| assert_eq!(s.borrow().as_ref().unwrap().scores.len(),1));
        result_json(); assert_eq!(output(),"null");
    }
}
