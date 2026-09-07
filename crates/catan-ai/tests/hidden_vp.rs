//! 伏せた勝利点カードが CPU に見えていないことの検証。
//!
//! 「自分の画面では勝利点が足された点数が見えるが、相手からは見えない」という
//! 前提でゲームを組んである。実際に**見えていない**ことを、
//! アクセサの有無ではなく **CPU の指し手が変わらないこと** で確かめる。

use catan_ai::bots::{Bot, GreedyEvalBot};
use catan_core::action::{Action, DevCard, Prompt};
use catan_core::game::{Game, GameConfig};
use catan_core::rng::Rng;
use catan_core::view::View;

/// 初期配置を済ませて、P0 の手番でダイスを振った直後まで進める
fn ready(seed: u64) -> Game {
    let mut g = Game::with_config(4, seed, GameConfig::default());
    let mut rng = Rng::with_stream(seed, 77);
    let mut buf = Vec::new();
    while g.is_setup() {
        g.legal_actions_into(&mut buf);
        g.apply(buf[rng.below(buf.len() as u32) as usize]);
    }
    while g.prompt != Prompt::PlayTurn {
        g.legal_actions_into(&mut buf);
        g.apply(buf[rng.below(buf.len() as u32) as usize]);
    }
    g
}

#[test]
fn 相手の伏せた勝利点はcpuの判断を変えない() {
    // 同じ局面で、相手 P1 の手札の発展カードを
    //   (a) 勝利点カード 2 枚
    //   (b) 騎士 2 枚
    // に変える。枚数は同じなので、**見えていないなら判断も同じはず**。
    let mut same = 0;
    for seed in 0..40u64 {
        let decide = |kind: DevCard| {
            let mut g = ready(seed);
            g.players[1].dev = [0; 5];
            g.players[1].dev[kind.idx()] = 2;
            let mut acts = Vec::new();
            g.legal_actions_into(&mut acts);
            let seat = g.to_act;
            let view = View::new(&g, seat);
            let mut bot = GreedyEvalBot::new(seed * 31 + 7);
            bot.decide(&view, &acts)
        };
        let a = decide(DevCard::VictoryPoint);
        let b = decide(DevCard::Knight);
        assert_eq!(a, b, "seed {seed}: 伏せたカードの種類で手が変わった = 見えている");
        same += 1;
    }
    assert_eq!(same, 40);
}

#[test]
fn 公開勝利点には勝利点カードが入らない() {
    let mut g = ready(3);
    let before_public = g.public_vp(1);
    let before_actual = g.actual_vp(1);
    g.players[1].dev[DevCard::VictoryPoint.idx()] += 2;
    assert_eq!(g.public_vp(1), before_public, "公開点に勝利点カードが漏れている");
    assert_eq!(g.actual_vp(1), before_actual + 2, "本当の点は増えるはず");

    // 相手席から見た View も公開点しか返さない
    let v = View::new(&g, 0);
    assert_eq!(v.public_vp(1), before_public);
    // 自分の点は伏せた分も込みで見える
    let v1 = View::new(&g, 1);
    assert_eq!(v1.my_actual_vp(), before_actual + 2);
}

#[test]
fn 勝利判定は伏せた点も数える() {
    // 見えないだけで、自分の手番に 10 点へ届けばその場で勝ちになる。
    // 勝利宣言は**自分の手番だけ**なので、手番を終えた後ではなく
    // 手番中の行動で判定される（EndTurn は既に次の人へ渡した後に判定される）。
    let mut g = ready(5);
    let p = g.turn_player;
    assert_eq!(g.to_act, p);
    let need = 10u8.saturating_sub(g.actual_vp(p));
    g.players[p as usize].dev[DevCard::VictoryPoint.idx()] += need;
    assert!(g.actual_vp(p) >= 10);
    assert!(g.winner.is_none(), "まだ何も指していない");

    g.apply(Action::Roll);
    assert_eq!(g.winner, Some(p), "伏せた勝利点で 10 点に届いたのに勝ちにならない");
}

#[test]
fn 伏せた勝利点は手番の外では勝ちにならない() {
    // 公式ルール: 勝利宣言できるのは自分の手番だけ
    let mut g = ready(9);
    let other = (g.turn_player + 1) % g.num_players;
    let need = 10u8.saturating_sub(g.actual_vp(other));
    g.players[other as usize].dev[DevCard::VictoryPoint.idx()] += need;
    assert!(g.actual_vp(other) >= 10);
    g.apply(Action::Roll);
    assert_ne!(g.winner, Some(other), "他人の手番中に勝ちになっている");
}
