//! ブラウザとの橋渡し。
//!
//! **wasm-bindgen も wasm-pack も npm も使わない。** 素の C ABI と JSON 文字列だけ。
//! エンジンが外部依存ゼロなので、そこはブラウザ側まで通してある
//! （`cargo build --target wasm32-unknown-unknown --release` で .wasm が出るだけ）。
//!
//! JS → Rust に構造を渡す必要が無いのが効いている。UI は合法手を**番号で**選ぶので、
//! Rust 側に JSON パーサが要らない。書き出す方向だけを手で書けばよい。

pub mod geometry;
mod json;

use catan_ai::bots::{bot_for_level, Bot};
use catan_core::action::{Action, Bundle, DevCard, Outcome, Prompt, EMPTY};
use catan_core::board::{BuildingKind, PlayerId, PortKind, Resource, RESOURCES};
use catan_core::game::{Game, GameConfig, TurnClock, MAX_PLAYERS};
use catan_core::topology::{EdgeId, NodeId, TileId, Topology, NUM_EDGES, NUM_NODES, NUM_TILES};
use catan_core::view::View;
use json::Json;
use std::cell::RefCell;

/// 棋譜の 1 行。誰の行いかを持たせて、プレイヤー別に絞れるようにする。
struct LogEntry {
    actor: PlayerId,
    text: String,
    /// 全体ログにも出す重要な手か（細かい交渉の往復は個人ログだけに置く）
    major: bool,
    /// この手で **払った** 資源。UI がカードの絵で出す
    cost: Bundle,
    /// この手で **得た** 資源
    gain: Bundle,
    /// 何手番目か（0 = 初期配置）。UI が「何巡目」でまとめるのに使う
    turn: u32,
}


/// 直前の 1 手。UI がアニメーションを描くのに要る情報だけを構造で渡す。
#[derive(Default)]
struct LastAction {
    kind: &'static str,
    actor: PlayerId,
    tile: Option<TileId>,
    from_tile: Option<TileId>,
    node: Option<NodeId>,
    edge: Option<EdgeId>,
    victim: Option<PlayerId>,
    /// 盗まれた側の、盗賊のヘクスに面した建物（カードが飛び出す位置）
    victim_node: Option<NodeId>,
    stolen: Option<Resource>,
    /// 手の通し番号。同じ手を二度アニメーションしないため
    seq: u32,
}

struct Session {
    game: Game,
    bots: Vec<Box<dyn Bot>>,
    /// 人間が座っている席（複数人でも回せるように配列で持つ）
    humans: Vec<bool>,
    actions: Vec<Action>,
    log: Vec<LogEntry>,
    last_dice: Option<(u8, u8)>,
    /// その対局で出た目の分布（添字 2..=12）。理論値と見比べられるように数えておく
    rolls: [u32; 13],
    last: LastAction,
    seq: u32,
    /// 出来事の順序番号（v2 CPU への配信用）
    evseq: u64,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

// ---------------------------------------------------------------- 文字列の受け渡し

fn put_out(s: String) -> *const u8 {
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        *o = s.into_bytes();
        o.as_ptr()
    })
}

/// 直前に書き出した文字列の長さ（バイト）
#[no_mangle]
pub extern "C" fn out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

// ---------------------------------------------------------------- ゲーム操作

/// 行動を適用し、出来事を受けたい CPU（v2）へ席ごとに投影して配る。
/// 人間の席のボットは作られていても使われないので、配っても無害。
fn apply_notify(sess: &mut Session, a: Action) -> catan_core::action::ActionRecord {
    let before = sess.game.clone();
    let rec = sess.game.apply(a);
    sess.evseq += 1;
    let seq = sess.evseq;
    for seat in 0..sess.game.n().min(sess.bots.len()) {
        if sess.bots[seat].wants_events() {
            let ev = catan_core::observer::project_event(&before, &sess.game, &rec, seat as u8, catan_core::observation::LEGACY_APP, seq);
            sess.bots[seat].observe(&ev);
        }
    }
    rec
}

/// 新しい対局を始める。`humans_mask` はビットで人間の席を指定（bit0 = 席0）。
///
/// 🔴 `humans_mask` と `ask_last_mask` は**別物**なので混ぜてはいけない。
///
/// - `humans_mask` = **手札の中身を見せてよい席**。オンラインでは自分の席だけ。
///   ここに他人を入れると、相手の手札が覗ける。
/// - `ask_last_mask` = **交渉で最後に聞く席**。規則の一部なので、
///   卓に着く全員（サーバを含む）で**同じ値**でなければならない。
///   ずれると `to_act` が食い違い、盤面がサーバとずれて進行不能になる。
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn game_new(
    board_seed: u32,
    dice_seed: u32,
    dev_seed: u32,
    steal_seed: u32,
    players: u32,
    humans_mask: u32,
    // levels: 席ごとの強さ。1 席 2 ビット（0=やさしい / 1=ふつう / 2=つよい / 3=さいきょう）
    levels: u32,
    ask_last_mask: u32,
) {
    let cfg = GameConfig {
        // ブラウザでは実時間。UI が tick で流し込む
        turn_clock: TurnClock::Realtime,
        // 人と遊ぶ時に持ち時間で急かさない。
        // 交渉が終わらなくなる心配は無い ── 断られた条件は同じ手番で出し直せず、
        // 生成される提案は 120 通りしかないので、いつか出し尽くして手番は終わる
        turn_time_limit_ms: None,
        // 提案の諾否は、**人間を最後に**聞く。
        // CPU は即答するので、人間が答える時点で他全員の返事が出そろっている
        // ＝ 一斉に聞かれたのと同じ見え方になる（colonist と同じ手触り）。
        //
        // 🔴 ここに `humans_mask`（＝自分の席だけ）を入れてはいけない。
        // オンラインでは席ごとに値が変わってしまい、サーバとも食い違って
        // `to_act` がずれる。呼ぶ側が卓で共通の値を渡す。
        answer_last_mask: ask_last_mask as u8,
        ..GameConfig::default()
    };
    let n = players.clamp(3, 4) as u8;
    // 出来事ごとに別の種。盤・サイコロ・発展カード・盗みが互いに影響しない
    let game = Game::with_streams(
        n,
        board_seed as u64,
        dice_seed as u64,
        dev_seed as u64,
        steal_seed as u64,
        cfg,
    );
    // 席ごとに強さを変えられる。`levels` は 1 席 2 ビット（0=やさしい 〜 3=さいきょう）。
    // 4 体を同じ卓に座らせた実測（1,500 戦・帰無仮説 25%）:
    //   やさしい 1.1% / ふつう 17.7% / つよい 31.1% / さいきょう 50.1%
    let bots: Vec<Box<dyn Bot>> = (0..n)
        .map(|i| {
            let lv = (levels >> (i * 2)) & 0b11;
            // 推論用の乱数は本番の出目・発展・盗みの種に依存させない。
            bot_for_level(lv, 0xC47A_2026 + i as u64)
        })
        .collect();
    let humans = (0..n).map(|i| humans_mask & (1 << i) != 0).collect();

    SESSION.with(|s| {
        *s.borrow_mut() = Some(Session {
            game,
            bots,
            humans,
            actions: Vec::new(),
            log: Vec::new(),
            last_dice: None,
            rolls: [0; 13],
            last: LastAction::default(),
            seq: 0,
            evseq: 0,
        })
    });
    refresh();
}

fn refresh() {
    SESSION.with(|s| {
        if let Some(sess) = s.borrow_mut().as_mut() {
            sess.game.legal_actions_into(&mut sess.actions);
        }
    });
}

/// いま手番のプレイヤーが人間か
#[no_mangle]
pub extern "C" fn is_human_turn() -> u32 {
    SESSION.with(|s| {
        let b = s.borrow();
        let Some(sess) = b.as_ref() else { return 0 };
        if sess.game.is_over() {
            return 0;
        }
        sess.humans
            .get(sess.game.to_act as usize)
            .copied()
            .unwrap_or(false) as u32
    })
}

/// `i` 番目の合法手を指す。成功なら 1。
#[no_mangle]
pub extern "C" fn apply_index(i: u32) -> u32 {
    SESSION.with(|s| {
        let mut b = s.borrow_mut();
        let Some(sess) = b.as_mut() else { return 0 };
        let Some(&a) = sess.actions.get(i as usize) else {
            return 0;
        };
        let before = sess.game.board.robber;
        let hands = hands_of(&sess.game);
        let rec = apply_notify(sess, a);
        note(sess, rec.actor, a, rec.result, before, hands);
        sess.game.legal_actions_into(&mut sess.actions);
        1
    })
}

/// CPU に 1 手指させる。指したら 1、手番が人間なら 0。
#[no_mangle]
pub extern "C" fn bot_step() -> u32 {
    SESSION.with(|s| {
        let mut b = s.borrow_mut();
        let Some(sess) = b.as_mut() else { return 0 };
        if sess.game.is_over() {
            return 0;
        }
        let seat = sess.game.to_act;
        if sess.humans.get(seat as usize).copied().unwrap_or(false) {
            return 0;
        }
        // 指す直前に作り直す。前の手からここまでの間に時計が進んでいることがある
        sess.game.legal_actions_into(&mut sess.actions);
        let a = {
            let view = View::new(&sess.game, seat);
            sess.bots[seat as usize].decide(&view, &sess.actions)
        };
        let before = sess.game.board.robber;
        let hands = hands_of(&sess.game);
        let rec = apply_notify(sess, a);
        note(sess, rec.actor, a, rec.result, before, hands);
        sess.game.legal_actions_into(&mut sess.actions);
        1
    })
}

/// 自分で組んだ提案を指す。合法なら 1。
///
/// 合法手の一覧に無い組み合わせでも、ルール上正しければエンジンは受ける
/// （禁じられているのは同種を両側に置くことだけで、枚数に制限は無い）。
/// UI の交易画面はここを使う。引数は資源 5 種の枚数（木・土・羊・麦・鉄）。
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn offer_custom(
    g0: u32, g1: u32, g2: u32, g3: u32, g4: u32,
    w0: u32, w1: u32, w2: u32, w3: u32, w4: u32,
) -> u32 {
    let give: Bundle = [g0 as u8, g1 as u8, g2 as u8, g3 as u8, g4 as u8];
    let want: Bundle = [w0 as u8, w1 as u8, w2 as u8, w3 as u8, w4 as u8];
    SESSION.with(|s| {
        let mut b = s.borrow_mut();
        let Some(sess) = b.as_mut() else { return 0 };
        if !offer_is_legal(&sess.game, sess.game.to_act, &give, &want) {
            return 0;
        }
        let a = Action::OfferTrade { give, want };
        let before = sess.game.board.robber;
        let hands = hands_of(&sess.game);
        let rec = apply_notify(sess, a);
        note(sess, rec.actor, a, rec.result, before, hands);
        sess.game.legal_actions_into(&mut sess.actions);
        1
    })
}

/// 自分で組んだ **対案** を返す。合法なら 1。
///
/// 提案と同じで、枚数に制限は無い（同種を両側に置くことだけが禁止）。
/// 引数は「自分が出す 5 種」と「自分が欲しい 5 種」。
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn counter_custom(
    g0: u32, g1: u32, g2: u32, g3: u32, g4: u32,
    w0: u32, w1: u32, w2: u32, w3: u32, w4: u32,
) -> u32 {
    let give: Bundle = [g0 as u8, g1 as u8, g2 as u8, g3 as u8, g4 as u8];
    let want: Bundle = [w0 as u8, w1 as u8, w2 as u8, w3 as u8, w4 as u8];
    SESSION.with(|s| {
        let mut b = s.borrow_mut();
        let Some(sess) = b.as_mut() else { return 0 };
        let g = &sess.game;
        if !matches!(g.prompt, Prompt::DecideTrade) || !g.negotiation_open() {
            return 0;
        }
        if !bundles_are_tradable(g, g.to_act, &give, &want) {
            return 0;
        }
        let a = Action::CounterOffer { give, want };
        let before = sess.game.board.robber;
        let hands = hands_of(&sess.game);
        let rec = apply_notify(sess, a);
        note(sess, rec.actor, a, rec.result, before, hands);
        sess.game.legal_actions_into(&mut sess.actions);
        1
    })
}

/// 対案の **候補を 1 つ積む**（返答はまだ確定しない）。積めたら 1。
///
/// 「木か土ならいいよ」を表すのに使う。候補を積んでから [`counter_custom`] を呼ぶと、
/// 積んだ分と合わせて 1 回の返答になり、**提案者がどれかを選ぶ**。
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn counter_alt(
    g0: u32, g1: u32, g2: u32, g3: u32, g4: u32,
    w0: u32, w1: u32, w2: u32, w3: u32, w4: u32,
) -> u32 {
    let give: Bundle = [g0 as u8, g1 as u8, g2 as u8, g3 as u8, g4 as u8];
    let want: Bundle = [w0 as u8, w1 as u8, w2 as u8, w3 as u8, w4 as u8];
    SESSION.with(|s| {
        let mut b = s.borrow_mut();
        let Some(sess) = b.as_mut() else { return 0 };
        let g = &sess.game;
        if !matches!(g.prompt, Prompt::DecideTrade) || !g.negotiation_open() {
            return 0;
        }
        if !bundles_are_tradable(g, g.to_act, &give, &want) {
            return 0;
        }
        // 枠が埋まっていたら受けない（UI 側に上限を伝えるため）
        let me = g.to_act as usize;
        if let Some(t) = &g.trade {
            if t.counters[me].iter().all(|c| c.is_some()) {
                return 0;
            }
        }
        let a = Action::CounterAlt { give, want };
        let before = sess.game.board.robber;
        let hands = hands_of(&sess.game);
        let rec = apply_notify(sess, a);
        note(sess, rec.actor, a, rec.result, before, hands);
        sess.game.legal_actions_into(&mut sess.actions);
        1
    })
}

/// 積んだ候補で返答を確定する。返せたら 1。
///
/// 積んである候補の最後の 1 本を [`Action::CounterOffer`] として指す。
/// `push_counter` は同じ条件を重ねないので、返答は **積んだ候補ちょうど** になる。
#[no_mangle]
pub extern "C" fn counter_finish() -> u32 {
    SESSION.with(|s| {
        let mut b = s.borrow_mut();
        let Some(sess) = b.as_mut() else { return 0 };
        let g = &sess.game;
        if !matches!(g.prompt, Prompt::DecideTrade) || !g.negotiation_open() {
            return 0;
        }
        let me = g.to_act as usize;
        let Some(t) = &g.trade else { return 0 };
        let Some((give, want)) = t.counters[me].iter().rev().flatten().next().copied() else {
            return 0; // 1 本も積んでいない
        };
        let a = Action::CounterOffer { give, want };
        let before = sess.game.board.robber;
        let hands = hands_of(&sess.game);
        let rec = apply_notify(sess, a);
        note(sess, rec.actor, a, rec.result, before, hands);
        sess.game.legal_actions_into(&mut sess.actions);
        1
    })
}

/// 積んだ対案の候補を 1 つ取り消す。消せたら 1。
#[no_mangle]
pub extern "C" fn counter_alt_remove(i: u32) -> u32 {
    SESSION.with(|s| {
        let mut b = s.borrow_mut();
        let Some(sess) = b.as_mut() else { return 0 };
        let g = &sess.game;
        if !matches!(g.prompt, Prompt::DecideTrade) {
            return 0;
        }
        let me = g.to_act as usize;
        match &g.trade {
            Some(t) if t.counters[me].get(i as usize).is_some_and(|c| c.is_some()) => {}
            _ => return 0,
        }
        let a = Action::CounterAltRemove(i as u8);
        let before = sess.game.board.robber;
        let hands = hands_of(&sess.game);
        let rec = apply_notify(sess, a);
        note(sess, rec.actor, a, rec.result, before, hands);
        sess.game.legal_actions_into(&mut sess.actions);
        1
    })
}

/// いま自分が積んでいる対案の候補数
#[no_mangle]
pub extern "C" fn counter_alt_count() -> u32 {
    SESSION.with(|s| {
        let b = s.borrow();
        let Some(sess) = b.as_ref() else { return 0 };
        let me = sess.game.to_act as usize;
        match &sess.game.trade {
            Some(t) => t.counters[me].iter().filter(|c| c.is_some()).count() as u32,
            None => 0,
        }
    })
}

/// いま対案を出せるか（交渉の持ち時間が残っているか）。出せるなら 1。
///
/// 「出せる組み合わせが自動生成されたか」で判定すると、生成の網から漏れただけの
/// 条件まで出せないことになる。UI は自前で条件を組むので、ここは**場面と時計だけ**を見る。
#[no_mangle]
pub extern "C" fn can_counter() -> u32 {
    SESSION.with(|s| {
        let b = s.borrow();
        let Some(sess) = b.as_ref() else { return 0 };
        let g = &sess.game;
        (matches!(g.prompt, Prompt::DecideTrade) && g.negotiation_open()) as u32
    })
}

/// 資源ごとの海上交易レート（4 / 3 / 2）。UI が「何枚で 1 枚か」を出すのに使う
#[no_mangle]
pub extern "C" fn maritime_rate(res: u32) -> u32 {
    SESSION.with(|s| {
        let b = s.borrow();
        let Some(sess) = b.as_ref() else { return 4 };
        let Some(r) = RESOURCES.get(res as usize).copied() else { return 4 };
        sess.game.board.best_maritime_rate(sess.game.to_act, r) as u32
    })
}

/// 銀行・港との交換を **まとめて** 行う。成立したら 1。
///
/// `MaritimeTrade` は 1 回で 1 枚しか貰えないので、8 枚出して 2 枚貰うには
/// 同じ操作を 2 度させることになる。ここでまとめて指せるようにする。
/// 中身は普通の海上交易を続けて指しているだけで、ルールは何も変えていない。
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn maritime_bulk(
    g0: u32, g1: u32, g2: u32, g3: u32, g4: u32,
    t0: u32, t1: u32, t2: u32, t3: u32, t4: u32,
) -> u32 {
    let give: Bundle = [g0 as u8, g1 as u8, g2 as u8, g3 as u8, g4 as u8];
    let take: Bundle = [t0 as u8, t1 as u8, t2 as u8, t3 as u8, t4 as u8];
    SESSION.with(|s| {
        let mut b = s.borrow_mut();
        let Some(sess) = b.as_mut() else { return 0 };
        let g = &sess.game;
        if !matches!(g.prompt, Prompt::PlayTurn) || !g.rolled {
            return 0;
        }
        let me = g.to_act;
        let hand = g.players[me as usize].hand;

        // 出す側はレートの倍数で、手札の範囲。貰う側と同じ資源は置けない
        let mut trades = 0u32;
        for (i, r) in RESOURCES.iter().enumerate() {
            if give[i] == 0 {
                continue;
            }
            if take[i] > 0 || give[i] > hand[i] {
                return 0;
            }
            let rate = g.board.best_maritime_rate(me, *r);
            if rate == 0 || give[i] % rate != 0 {
                return 0;
            }
            trades += (give[i] / rate) as u32;
        }
        let want: u32 = take.iter().map(|&n| n as u32).sum();
        if trades == 0 || want != trades {
            return 0;
        }
        // 銀行に在庫があるか
        for i in 0..5 {
            if take[i] > g.bank[i] {
                return 0;
            }
        }

        // ここまで通れば、あとは普通の海上交易を順に指すだけ
        let before_robber = sess.game.board.robber;
        let hands = hands_of(&sess.game);
        let mut queue: Vec<Resource> = Vec::new();
        for (i, r) in RESOURCES.iter().enumerate() {
            for _ in 0..take[i] {
                queue.push(*r);
            }
            let _ = r;
        }
        let mut qi = 0;
        let mut last = Action::EndTurn;
        for (i, r) in RESOURCES.iter().enumerate() {
            if give[i] == 0 {
                continue;
            }
            let rate = sess.game.board.best_maritime_rate(me, *r);
            for _ in 0..(give[i] / rate) {
                let a = Action::MaritimeTrade { give: *r, count: rate, take: queue[qi] };
                qi += 1;
                apply_notify(sess, a);
                last = a;
            }
        }

        // 棋譜は 1 行にまとめる（1 回の操作なので）
        sess.seq += 1;
        sess.last = LastAction {
            kind: action_kind(&last),
            actor: me,
            seq: sess.seq,
            ..LastAction::default()
        };
        let (cost, gain) = split_delta(&hands[me as usize], &sess.game.players[me as usize].hand);
        let turn = sess.game.turn;
        sess.log.push(LogEntry {
            actor: me,
            text: format!("銀行と {} → {}", bundle_text(&give), bundle_text(&take)),
            major: true,
            cost,
            gain,
            turn,
        });
        let _ = before_robber;
        sess.game.legal_actions_into(&mut sess.actions);
        1
    })
}

/// いま自分が捨てなければならない枚数（0 なら捨てる場面ではない）
#[no_mangle]
pub extern "C" fn discard_needed() -> u32 {
    SESSION.with(|s| {
        let b = s.borrow();
        let Some(sess) = b.as_ref() else { return 0 };
        sess.game.discard_pending[sess.game.to_act as usize] as u32
    })
}

/// 自分で選んだ組み合わせで捨てる。捨てられたら 1。
///
/// `legal_actions` は代表例に間引いた候補しか返さないが、`apply` は
/// 「持っていて・枚数がちょうど」なら任意の組み合わせを受ける。UI はここを使う。
#[no_mangle]
pub extern "C" fn discard_custom(d0: u32, d1: u32, d2: u32, d3: u32, d4: u32) -> u32 {
    let b: Bundle = [d0 as u8, d1 as u8, d2 as u8, d3 as u8, d4 as u8];
    SESSION.with(|s| {
        let mut bo = s.borrow_mut();
        let Some(sess) = bo.as_mut() else { return 0 };
        let g = &sess.game;
        if !matches!(g.prompt, Prompt::Discard) {
            return 0;
        }
        let me = g.to_act as usize;
        let need = g.discard_pending[me];
        let hand = g.players[me].hand;
        if b.iter().map(|&n| n as u32).sum::<u32>() != need as u32 {
            return 0;
        }
        if (0..5).any(|i| b[i] > hand[i]) {
            return 0;
        }
        let a = Action::Discard(b);
        let before = sess.game.board.robber;
        let hands = hands_of(&sess.game);
        let rec = apply_notify(sess, a);
        note(sess, rec.actor, a, rec.result, before, hands);
        sess.game.legal_actions_into(&mut sess.actions);
        1
    })
}

/// 束としてルール上成立するか（持っている・両側 1 枚以上・同種を含まない）
fn bundles_are_tradable(g: &Game, p: PlayerId, give: &Bundle, want: &Bundle) -> bool {
    let hand = g.players[p as usize].hand;
    let total = |b: &Bundle| b.iter().map(|&n| n as u32).sum::<u32>();
    // 片側が空でもよい ── colonist の「?」の札（中身は相手に決めてもらう）。
    // 両側とも空は何も動かないので不可
    (total(give) > 0 || total(want) > 0)
        && (0..5).all(|i| give[i] <= hand[i])
        // 同種を両側に置くと「ただで貰う」形になるので禁止
        && (0..5).all(|i| give[i] == 0 || want[i] == 0)
}

/// その提案がルール上通るか。`apply` は反則で panic するので、UI は先にここで弾く
fn offer_is_legal(g: &Game, p: PlayerId, give: &Bundle, want: &Bundle) -> bool {
    bundles_are_tradable(g, p, give, want)
        && g.negotiation_open()
        && matches!(g.prompt, Prompt::PlayTurn)
        && g.rolled
}

/// 実時間を流し込む（手番の持ち時間用）
#[no_mangle]
pub extern "C" fn tick(ms: u32) {
    SESSION.with(|s| {
        if let Some(sess) = s.borrow_mut().as_mut() {
            // 🔴 交渉の持ち時間が切れると提案・対案は非合法になる。
            // 手の一覧を作り直さないと、切れる前に作った提案をそのまま指してしまい
            // `apply` の検査で落ちる（ブラウザで実際に panic した）。
            let open_before = sess.game.negotiation_open();
            sess.game.tick(ms);
            if open_before && !sess.game.negotiation_open() {
                sess.game.legal_actions_into(&mut sess.actions);
            }
        }
    });
}

// ---------------------------------------------------------------- 書き出し

/// 盤の形（対局中は変わらない）
#[no_mangle]
pub extern "C" fn board_json() -> *const u8 {
    SESSION.with(|s| {
        let b = s.borrow();
        let Some(sess) = b.as_ref() else {
            return put_out("null".into());
        };
        let g = &sess.game;
        let topo = Topology::get();
        let mut j = Json::new();
        j.obj_start();

        let (x0, y0, x1, y1) = geometry::bounds();
        j.key("viewBox");
        j.raw(&format!("[{x0:.1},{y0:.1},{:.1},{:.1}]", x1 - x0, y1 - y0));
        j.key("hexSize");
        j.num(geometry::HEX_SIZE);

        j.key("tiles");
        j.arr_start();
        for t in 0..NUM_TILES {
            let (x, y) = geometry::tile_center(t as u8);
            j.obj_start();
            j.key("id");
            j.num(t as f32);
            j.key("x");
            j.num(x);
            j.key("y");
            j.num(y);
            j.key("resource");
            match g.board.tile_resource[t] {
                Some(r) => j.str(res_key(r)),
                None => j.raw("null"),
            }
            j.key("number");
            j.num(g.board.tile_number[t] as f32);
            j.key("pips");
            j.num(g.board.tile_pips(t as u8) as f32);
            // そのヘクスに面する 6 頂点。盗賊で「誰に面しているか」を UI が出すのに要る
            j.key("nodes");
            j.arr_start();
            for n in Topology::get().tile_nodes[t] {
                j.num(n as f32);
            }
            j.arr_end();
            j.obj_end();
        }
        j.arr_end();

        j.key("nodes");
        j.arr_start();
        for n in 0..NUM_NODES {
            let (x, y) = geometry::node_pos(n as NodeId);
            j.obj_start();
            j.key("id");
            j.num(n as f32);
            j.key("x");
            j.num(x);
            j.key("y");
            j.num(y);
            j.obj_end();
        }
        j.arr_end();

        j.key("edges");
        j.arr_start();
        for e in 0..NUM_EDGES {
            let ((ax, ay), (bx, by)) = geometry::edge_ends(e as EdgeId);
            let ends = Topology::get().edge_nodes[e];
            j.obj_start();
            j.key("id");
            j.num(e as f32);
            // 端点の頂点番号。UI が「どちら側から道を伸ばしたか」を出すのに要る
            j.key("n1");
            j.num(ends[0] as f32);
            j.key("n2");
            j.num(ends[1] as f32);
            j.key("x1");
            j.num(ax);
            j.key("y1");
            j.num(ay);
            j.key("x2");
            j.num(bx);
            j.key("y2");
            j.num(by);
            j.obj_end();
        }
        j.arr_end();

        j.key("ports");
        j.arr_start();
        for e in catan_core::board::port_edges() {
            let n = topo.edge_nodes[e as usize][0];
            let Some(kind) = g.board.node_port[n as usize] else {
                continue;
            };
            let (x, y) = geometry::port_marker(e);
            let ((ax, ay), (bx, by)) = geometry::edge_ends(e);
            j.obj_start();
            j.key("edge");
            j.num(e as f32);
            j.key("x");
            j.num(x);
            j.key("y");
            j.num(y);
            j.key("ax");
            j.num(ax);
            j.key("ay");
            j.num(ay);
            j.key("bx");
            j.num(bx);
            j.key("by");
            j.num(by);
            j.key("kind");
            match kind {
                PortKind::Generic => j.str("ANY"),
                PortKind::Specific(r) => j.str(res_key(r)),
            }
            j.obj_end();
        }
        j.arr_end();

        j.obj_end();
        put_out(j.finish())
    })
}

/// いまの局面（毎手ごとに読み直す）
#[no_mangle]
pub extern "C" fn state_json() -> *const u8 {
    SESSION.with(|s| {
        let b = s.borrow();
        let Some(sess) = b.as_ref() else {
            return put_out("null".into());
        };
        let g = &sess.game;
        let mut j = Json::new();
        j.obj_start();

        j.key("prompt");
        j.str(prompt_key(g.prompt));
        j.key("promptText");
        j.str(&prompt_text(g));
        j.key("toAct");
        j.num(g.to_act as f32);
        j.key("turnPlayer");
        j.num(g.turn_player as f32);
        j.key("turn");
        j.num(g.turn as f32);
        j.key("rolled");
        j.bool(g.rolled);
        j.key("isSetup");
        j.bool(g.is_setup());
        j.key("robber");
        j.num(g.board.robber as f32);
        // 出た目の分布（2..=12）。偏っているのか、そう見えるだけなのかを見分けるため
        j.key("rollCounts");
        j.arr_start();
        for n in 2..=12 {
            j.num(sess.rolls[n] as f32);
        }
        j.arr_end();

        j.key("devLeft");
        j.num(g.dev_deck.iter().sum::<u8>() as f32);
        j.key("numPlayers");
        j.num(g.num_players as f32);
        j.key("winner");
        match g.winner {
            Some(w) => j.num(w as f32),
            None => j.raw("null"),
        }
        j.key("dice");
        match sess.last_dice {
            Some((a, b)) => j.raw(&format!("[{a},{b}]")),
            None => j.raw("null"),
        }
        j.key("longestRoadOwner");
        match g.longest_road_owner {
            Some(p) => j.num(p as f32),
            None => j.raw("null"),
        }
        j.key("largestArmyOwner");
        match g.largest_army_owner {
            Some(p) => j.num(p as f32),
            None => j.raw("null"),
        }
        j.key("turnTimeLeftMs");
        match g.turn_time_left_ms() {
            Some(v) => j.num(v as f32),
            None => j.raw("null"),
        }

        j.key("bank");
        j.raw(&format!(
            "[{},{},{},{},{}]",
            g.bank[0], g.bank[1], g.bank[2], g.bank[3], g.bank[4]
        ));

        // --- 建物と道 ---
        j.key("buildings");
        j.arr_start();
        for n in 0..NUM_NODES {
            if let Some(bl) = g.board.building[n] {
                j.obj_start();
                j.key("node");
                j.num(n as f32);
                j.key("owner");
                j.num(bl.owner as f32);
                j.key("kind");
                j.str(match bl.kind {
                    BuildingKind::Settlement => "SETTLEMENT",
                    BuildingKind::City => "CITY",
                });
                j.obj_end();
            }
        }
        j.arr_end();

        j.key("roads");
        j.arr_start();
        for e in 0..NUM_EDGES {
            if let Some(o) = g.board.road[e] {
                j.obj_start();
                j.key("edge");
                j.num(e as f32);
                j.key("owner");
                j.num(o as f32);
                j.obj_end();
            }
        }
        j.arr_end();

        // --- プレイヤー ---
        j.key("players");
        j.arr_start();
        for q in 0..g.n() {
            let p = q as PlayerId;
            let ps = &g.players[q];
            let human = sess.humans.get(q).copied().unwrap_or(false);
            j.obj_start();
            j.key("id");
            j.num(q as f32);
            j.key("human");
            j.bool(human);
            j.key("publicVp");
            j.num(g.public_vp(p) as f32);
            j.key("handSize");
            j.num(ps.hand_size() as f32);
            j.key("devCount");
            j.num(ps.dev_count() as f32);
            j.key("playedKnights");
            j.num(ps.played_knights() as f32);
            j.key("longestRoad");
            j.num(ps.longest_road as f32);
            j.key("roadsLeft");
            j.num(ps.roads_left as f32);
            j.key("settlementsLeft");
            j.num(ps.settlements_left as f32);
            j.key("citiesLeft");
            j.num(ps.cities_left as f32);
            // 手札の中身と実際の勝利点は、その席が人間か決着後だけ
            let reveal = human || g.is_over();
            j.key("hand");
            if reveal {
                j.raw(&format!(
                    "[{},{},{},{},{}]",
                    ps.hand[0], ps.hand[1], ps.hand[2], ps.hand[3], ps.hand[4]
                ));
            } else {
                j.raw("null");
            }
            j.key("dev");
            if reveal {
                j.raw(&format!(
                    "[{},{},{},{},{}]",
                    ps.dev[0], ps.dev[1], ps.dev[2], ps.dev[3], ps.dev[4]
                ));
            } else {
                j.raw("null");
            }
            j.key("vp");
            if reveal {
                j.num(g.actual_vp(p) as f32);
            } else {
                j.raw("null");
            }
            // 勝利点の内訳。決着後の結果表に出す（伏せた分は決着まで隠す）
            j.key("vpParts");
            if g.is_over() {
                let topo = Topology::get();
                let (mut settlements, mut cities) = (0u8, 0u8);
                for n in 0..NUM_NODES {
                    if let Some(b) = g.board.building[n] {
                        if b.owner == p {
                            match b.kind {
                                BuildingKind::Settlement => settlements += 1,
                                BuildingKind::City => cities += 1,
                            }
                        }
                    }
                }
                let _ = topo;
                j.obj_start();
                j.key("settlements");
                j.num(settlements as f32);
                j.key("cities");
                j.num(cities as f32);
                j.key("longestRoad");
                j.bool(g.longest_road_owner == Some(p));
                j.key("largestArmy");
                j.bool(g.largest_army_owner == Some(p));
                j.key("devVp");
                j.num(ps.dev[DevCard::VictoryPoint.idx()] as f32);
                j.obj_end();
            } else {
                j.raw("null");
            }
            j.obj_end();
        }
        j.arr_end();

        // --- 交渉中の内容 ---
        j.key("trade");
        match g.trade {
            None => j.raw("null"),
            Some(t) => {
                j.obj_start();
                j.key("proposer");
                j.num(t.proposer as f32);
                j.key("give");
                j.raw(&bundle_json(&t.give));
                j.key("want");
                j.raw(&bundle_json(&t.want));
                // 誰がどう答えたか。UI が交渉の場を丸ごと見せるのに要る
                j.key("answers");
                j.arr_start();
                for p in 0..g.num_players as usize {
                    j.obj_start();
                    j.key("p");
                    j.num(p as f32);
                    if p == t.proposer as usize {
                        j.key("state");
                        j.str("PROPOSER");
                    } else if t.counters[p].iter().any(|c| c.is_some()) {
                        j.key("state");
                        j.str("COUNTER");
                        // 候補は複数ありうる（「木か土ならいい」）。相手目線の (出す, 欲しい)
                        j.key("alts");
                        j.arr_start();
                        for (k, c) in t.counters[p].iter().enumerate() {
                            if let Some((cg, cw)) = c {
                                j.obj_start();
                                j.key("alt");
                                j.num(k as f32);
                                j.key("give");
                                j.raw(&bundle_json(cg));
                                j.key("want");
                                j.raw(&bundle_json(cw));
                                j.obj_end();
                            }
                        }
                        j.arr_end();
                    } else if t.accepted[p] {
                        j.key("state");
                        j.str("ACCEPTED");
                    } else if t.responded[p] {
                        // 🔴 以前は「提案者の次から席順に回る」前提で順位から推測していたが、
                        //    人間を最後に回すようにしたので席順では当たらない。
                        //    誰が答えたかは `responded` を直接見る
                        j.key("state");
                        j.str("REJECTED");
                    } else {
                        j.key("state");
                        j.str("WAITING");
                    }
                    j.obj_end();
                }
                j.arr_end();
                j.obj_end();
            }
        }

        // --- 選べる手 ---
        j.key("actions");
        j.arr_start();
        for (i, a) in sess.actions.iter().enumerate() {
            j.obj_start();
            j.key("i");
            j.num(i as f32);
            j.key("kind");
            j.str(action_kind(a));
            j.key("label");
            j.str(&action_label(g, a));
            // 盤上のどこを指す手か（クリック対象の対応づけ）
            match a {
                Action::SetupSettlement(n) | Action::BuildSettlement(n) | Action::BuildCity(n) => {
                    j.key("node");
                    j.num(*n as f32);
                }
                Action::SetupRoad(e) | Action::BuildRoad(e) => {
                    j.key("edge");
                    j.num(*e as f32);
                }
                Action::MoveRobber { tile, victim } => {
                    j.key("tile");
                    j.num(*tile as f32);
                    if let Some(v) = victim {
                        j.key("victim");
                        j.num(*v as f32);
                    }
                }
                // 発展カードで選ぶ資源。UI がカードの絵で出せるように番号で渡す
                Action::PlayMonopoly(r) => {
                    j.key("res");
                    j.num(r.idx() as f32);
                }
                Action::PlayYearOfPlenty(a2, b2) => {
                    j.key("res");
                    j.num(a2.idx() as f32);
                    j.key("res2");
                    j.num(b2.idx() as f32);
                }
                Action::Discard(b) => {
                    j.key("give");
                    j.raw(&bundle_json(b));
                }
                // 交易は「何を出して何を貰うか」を絵で出したいので中身を渡す
                Action::OfferTrade { give, want } | Action::CounterOffer { give, want } => {
                    j.key("give");
                    j.raw(&bundle_json(give));
                    j.key("want");
                    j.raw(&bundle_json(want));
                }
                Action::MaritimeTrade { give, count, take } => {
                    let mut g = EMPTY;
                    g[give.idx()] = *count;
                    let mut w = EMPTY;
                    w[take.idx()] = 1;
                    j.key("give");
                    j.raw(&bundle_json(&g));
                    j.key("want");
                    j.raw(&bundle_json(&w));
                }
                // 相手の返答を選ぶ手。誰の・どんな条件かを添える
                Action::ConfirmTrade(p) => {
                    j.key("who");
                    j.num(*p as f32);
                    if let Some(t) = &g.trade {
                        // 承諾なので条件は提案そのもの。向きは「自分が出す → 貰う」
                        j.key("give");
                        j.raw(&bundle_json(&t.give));
                        j.key("want");
                        j.raw(&bundle_json(&t.want));
                    }
                }
                Action::AcceptCounter { from: p, alt } => {
                    j.key("who");
                    j.num(*p as f32);
                    j.key("alt");
                    j.num(*alt as f32);
                    if let Some((cg, cw)) = g
                        .trade
                        .as_ref()
                        .and_then(|t| t.counters[*p as usize][*alt as usize])
                    {
                        // 対案は相手目線なので、こちらから見ると give/want が入れ替わる
                        j.key("give");
                        j.raw(&bundle_json(&cw));
                        j.key("want");
                        j.raw(&bundle_json(&cg));
                    }
                }
                _ => {}
            }
            j.obj_end();
        }
        j.arr_end();

        j.key("log");
        j.arr_start();
        for e in sess.log.iter().rev().take(120) {
            j.obj_start();
            j.key("actor");
            j.num(e.actor as f32);
            j.key("text");
            j.str(&e.text);
            j.key("major");
            j.bool(e.major);
            j.key("turn");
            j.num(e.turn as f32);
            if e.cost != EMPTY {
                j.key("cost");
                j.raw(&bundle_json(&e.cost));
            }
            if e.gain != EMPTY {
                j.key("gain");
                j.raw(&bundle_json(&e.gain));
            }
            j.obj_end();
        }
        j.arr_end();

        // --- 直前の 1 手（アニメーション用）---
        j.key("last");
        {
            let l = &sess.last;
            j.obj_start();
            j.key("seq");
            j.num(l.seq as f32);
            j.key("kind");
            j.str(l.kind);
            j.key("actor");
            j.num(l.actor as f32);
            for (k, v) in [
                ("tile", l.tile.map(|v| v as f32)),
                ("fromTile", l.from_tile.map(|v| v as f32)),
                ("node", l.node.map(|v| v as f32)),
                ("edge", l.edge.map(|v| v as f32)),
                ("victim", l.victim.map(|v| v as f32)),
                ("victimNode", l.victim_node.map(|v| v as f32)),
            ] {
                j.key(k);
                match v {
                    Some(x) => j.num(x),
                    None => j.raw("null"),
                }
            }
            // 奪った資源の種類は当事者（奪った人・奪われた人）にだけ見せる。
            // 第三者の席には「1 枚移った」ことだけ（裏向きの札）を伝える
            j.key("stolen");
            let human = |q: PlayerId| sess.humans.get(q as usize).copied().unwrap_or(false);
            let party = human(l.actor) || l.victim.map_or(false, human);
            match l.stolen {
                Some(r) if party || g.is_over() => j.str(res_key(r)),
                Some(_) => j.str("HIDDEN"),
                None => j.raw("null"),
            }
            j.obj_end();
        }

        j.obj_end();
        put_out(j.finish())
    })
}

// ---------------------------------------------------------------- 表示用の文字列

fn bundle_json(b: &[u8; 5]) -> String {
    format!("[{},{},{},{},{}]", b[0], b[1], b[2], b[3], b[4])
}

fn res_key(r: Resource) -> &'static str {
    match r {
        Resource::Wood => "WOOD",
        Resource::Brick => "BRICK",
        Resource::Sheep => "SHEEP",
        Resource::Wheat => "WHEAT",
        Resource::Ore => "ORE",
    }
}

fn bundle_text(b: &[u8; 5]) -> String {
    let mut parts = Vec::new();
    for r in RESOURCES {
        let n = b[r.idx()];
        if n > 0 {
            parts.push(format!("{}{}", r.ja(), n));
        }
    }
    if parts.is_empty() {
        "なし".into()
    } else {
        parts.join("・")
    }
}

fn prompt_key(p: Prompt) -> &'static str {
    match p {
        Prompt::SetupSettlement => "SETUP_SETTLEMENT",
        Prompt::SetupRoad => "SETUP_ROAD",
        Prompt::PlayTurn => "PLAY_TURN",
        Prompt::Discard => "DISCARD",
        Prompt::MoveRobber => "MOVE_ROBBER",
        Prompt::FreeRoad => "FREE_ROAD",
        Prompt::DecideTrade => "DECIDE_TRADE",
        Prompt::DecideAcceptees => "DECIDE_ACCEPTEES",
        Prompt::GameOver => "GAME_OVER",
    }
}

fn prompt_text(g: &Game) -> String {
    match g.prompt {
        Prompt::SetupSettlement => "開拓地を置く場所を選んでください".into(),
        Prompt::SetupRoad => "道を置く場所を選んでください".into(),
        Prompt::PlayTurn if !g.rolled => "サイコロを振ってください".into(),
        Prompt::PlayTurn => "建設・交易・手番終了".into(),
        Prompt::Discard => format!(
            "7 が出ました。{} 枚捨ててください",
            g.discard_pending[g.to_act as usize]
        ),
        Prompt::MoveRobber => "盗賊を動かす場所を選んでください".into(),
        Prompt::FreeRoad => format!("無償の道をあと {} 本置けます", g.free_roads),
        Prompt::DecideTrade => {
            let t = g.trade.expect("交易中");
            format!(
                "P{} の提案: {} を渡すので {} が欲しい",
                t.proposer,
                bundle_text(&t.give),
                bundle_text(&t.want)
            )
        }
        Prompt::DecideAcceptees => "交易の相手を選んでください".into(),
        Prompt::GameOver => match g.winner {
            Some(w) => format!("P{w} の勝ち"),
            None => "終了".into(),
        },
    }
}

fn action_kind(a: &Action) -> &'static str {
    match a {
        Action::SetupSettlement(_) => "SETUP_SETTLEMENT",
        Action::SetupRoad(_) => "SETUP_ROAD",
        Action::Roll => "ROLL",
        Action::EndTurn => "END_TURN",
        Action::BuildRoad(_) => "BUILD_ROAD",
        Action::BuildSettlement(_) => "BUILD_SETTLEMENT",
        Action::BuildCity(_) => "BUILD_CITY",
        Action::BuyDevCard => "BUY_DEV",
        Action::PlayKnight => "PLAY_KNIGHT",
        Action::PlayRoadBuilding => "PLAY_ROAD_BUILDING",
        Action::PlayYearOfPlenty(..) => "PLAY_YEAR_OF_PLENTY",
        Action::PlayMonopoly(_) => "PLAY_MONOPOLY",
        Action::Discard(_) => "DISCARD",
        Action::MoveRobber { .. } => "MOVE_ROBBER",
        Action::MaritimeTrade { .. } => "MARITIME_TRADE",
        Action::OfferTrade { .. } => "OFFER_TRADE",
        Action::AcceptTrade => "ACCEPT_TRADE",
        Action::RejectTrade => "REJECT_TRADE",
        Action::CounterAlt { .. } => "COUNTER_ALT",
        Action::CounterAltRemove(_) => "COUNTER_ALT_REMOVE",
        Action::CounterOffer { .. } => "COUNTER_OFFER",
        Action::ConfirmTrade(_) => "CONFIRM_TRADE",
        Action::AcceptCounter { .. } => "ACCEPT_COUNTER",
        Action::CancelTrade => "CANCEL_TRADE",
    }
}

fn action_label(g: &Game, a: &Action) -> String {
    match a {
        Action::SetupSettlement(_) | Action::BuildSettlement(_) => "開拓地を建てる".into(),
        Action::SetupRoad(_) | Action::BuildRoad(_) => "道を建てる".into(),
        Action::BuildCity(_) => "都市にする".into(),
        Action::Roll => "サイコロを振る".into(),
        Action::EndTurn => "手番を終える".into(),
        Action::BuyDevCard => "発展カードを買う".into(),
        Action::PlayKnight => "騎士を使う".into(),
        Action::PlayRoadBuilding => "街道建設を使う".into(),
        Action::PlayYearOfPlenty(x, y) => format!("収穫: {}・{}", x.ja(), y.ja()),
        Action::PlayMonopoly(r) => format!("独占: {}", r.ja()),
        Action::Discard(b) => format!("捨てる: {}", bundle_text(b)),
        Action::MoveRobber { tile, victim } => match victim {
            Some(v) => format!("盗賊をヘクス{tile}へ / P{v} から盗む"),
            None => format!("盗賊をヘクス{tile}へ"),
        },
        Action::MaritimeTrade { give, count, take } => {
            format!("銀行と {}{}→{}1", give.ja(), count, take.ja())
        }
        Action::OfferTrade { give, want } => {
            format!("提案: {} → {}", bundle_text(give), bundle_text(want))
        }
        Action::CounterAlt { give, want } => {
            format!("候補: {} → {}", bundle_text(give), bundle_text(want))
        }
        Action::CounterAltRemove(_) => "候補を取り消す".into(),
        Action::CounterOffer { give, want } => {
            format!("対案: {} → {}", bundle_text(give), bundle_text(want))
        }
        Action::AcceptTrade => "受ける".into(),
        Action::RejectTrade => "断る".into(),
        Action::ConfirmTrade(p) => format!("P{p} と成立させる"),
        Action::AcceptCounter { from, .. } => format!("P{from} の対案を受ける"),
        Action::CancelTrade => {
            let _ = g;
            "取り下げる".into()
        }
    }
}

/// 盗賊のヘクスに面した `victim` の建物（カードが飛び出す位置）
fn victim_building(game: &Game, tile: TileId, victim: PlayerId) -> Option<NodeId> {
    let topo = Topology::get();
    topo.tile_nodes[tile as usize]
        .into_iter()
        .find(|&n| matches!(game.board.building[n as usize], Some(b) if b.owner == victim))
}

/// 全員の手札の写し。行動の前後で引き算して「何が動いたか」を出すのに使う
fn hands_of(game: &Game) -> [Bundle; MAX_PLAYERS] {
    let mut h = [EMPTY; MAX_PLAYERS];
    for (i, p) in game.players.iter().enumerate() {
        h[i] = p.hand;
    }
    h
}

/// `after − before` を「払った分」と「得た分」に割る
fn split_delta(before: &Bundle, after: &Bundle) -> (Bundle, Bundle) {
    let mut cost = EMPTY;
    let mut gain = EMPTY;
    for i in 0..cost.len() {
        cost[i] = before[i].saturating_sub(after[i]);
        gain[i] = after[i].saturating_sub(before[i]);
    }
    (cost, gain)
}

/// 棋譜に 1 行足し、アニメーション用の情報を控える。
///
/// 資源の増減は行動ごとに書き分けず、**手札の前後の差**から出す。
/// こうしておけば交易・海上交易・建設・産出のどれでも同じ 1 か所で正しくなる。
fn note(
    sess: &mut Session,
    actor: PlayerId,
    a: Action,
    out: Outcome,
    before_robber: TileId,
    before_hands: [Bundle; MAX_PLAYERS],
) {
    sess.seq += 1;
    let mut last = LastAction {
        kind: action_kind(&a),
        actor,
        seq: sess.seq,
        ..LastAction::default()
    };
    let mut major = true;

    let line = match (a, out) {
        (Action::Roll, Outcome::Dice(x, y)) => {
            sess.last_dice = Some((x, y));
            sess.rolls[(x + y) as usize] += 1;
            format!("サイコロ {x}+{y}={}", x + y)
        }
        (Action::MoveRobber { tile, victim }, Outcome::Stole(st)) => {
            last.tile = Some(tile);
            last.from_tile = Some(before_robber);
            last.victim = victim;
            last.stolen = st;
            if let Some(v) = victim {
                last.victim_node = victim_building(&sess.game, tile, v);
            }
            // 何を奪ったかは書かない（本来は当人どうししか知らない情報）
            match (victim, st) {
                (Some(v), Some(_)) => format!("盗賊を動かし P{v} から 1 枚奪った"),
                (Some(v), None) => format!("盗賊を動かした（P{v} の手札が空）"),
                (None, _) => "盗賊を動かした（奪える相手なし）".into(),
            }
        }
        (Action::SetupSettlement(n) | Action::BuildSettlement(n), _) => {
            last.node = Some(n);
            "開拓地を建てた".into()
        }
        (Action::BuildCity(n), _) => {
            last.node = Some(n);
            "都市にした".into()
        }
        (Action::SetupRoad(e) | Action::BuildRoad(e), _) => {
            last.edge = Some(e);
            "道を建てた".into()
        }
        (Action::BuyDevCard, _) => "発展カードを買った".into(),
        (Action::PlayKnight, _) => "騎士を出した".into(),
        (Action::EndTurn, _) => return,
        (Action::RejectTrade, _) => {
            major = false;
            "交易を断った".into()
        }
        (Action::OfferTrade { give, want }, _) => {
            major = false;
            format!("提案: {} → {}", bundle_text(&give), bundle_text(&want))
        }
        (Action::CounterOffer { give, want }, _) => {
            major = false;
            format!("対案: {} → {}", bundle_text(&give), bundle_text(&want))
        }
        // 候補の積み下ろしは交渉の下書き。全体ログには出さない
        (Action::CounterAlt { give, want }, _) => {
            major = false;
            format!("候補: {} → {}", bundle_text(&give), bundle_text(&want))
        }
        (Action::CounterAltRemove(_), _) => {
            major = false;
            "候補を取り消した".into()
        }
        (Action::AcceptTrade, _) => {
            major = false;
            "交易を受けた".into()
        }
        (Action::CancelTrade, _) => {
            major = false;
            "交易を取り下げた".into()
        }
        (Action::ConfirmTrade(p), _) => {
            format!("P{p} と交易が成立")
        }
        (Action::AcceptCounter { from: p, .. }, _) => {
            format!("P{p} と交易が成立")
        }
        // 銀行との交換はカードが動く。交渉の往復とは違って記録に残す
        (Action::MaritimeTrade { give, count, take }, _) => {
            format!("銀行と {}{} → {}1", give.ja(), count, take.ja())
        }
        (Action::Discard(b), _) => format!("{} を捨てた", bundle_text(&b)),
        _ => action_label(&sess.game, &a),
    };

    sess.last = last;

    // 資源の動きを手札の差から出す。
    // 盗賊・廃棄・交渉の往復は文面がすでに中身を書いているので絵は付けない。
    let show_flow = !matches!(
        a,
        Action::MoveRobber { .. }
            | Action::Discard(_)
            | Action::OfferTrade { .. }
            | Action::CounterOffer { .. }
            | Action::RejectTrade
            | Action::AcceptTrade
            | Action::CancelTrade
    );
    let (cost, gain) = if show_flow {
        split_delta(&before_hands[actor as usize], &sess.game.players[actor as usize].hand)
    } else {
        (EMPTY, EMPTY)
    };
    let turn = sess.game.turn;
    sess.log.push(LogEntry { actor, text: line, major, cost, gain, turn });

    // サイコロの後は「誰が何を得たか」を人数ぶん並べる。
    // 手番の人の分は上の行に付いているので、それ以外を出す。
    if matches!(a, Action::Roll) {
        for p in 0..sess.game.num_players {
            if p == actor {
                continue;
            }
            let (_, g) = split_delta(&before_hands[p as usize], &sess.game.players[p as usize].hand);
            if g != EMPTY {
                sess.log.push(LogEntry {
                    actor: p,
                    text: "収穫".into(),
                    major: true,
                    cost: EMPTY,
                    gain: g,
                    turn,
                });
            }
        }
    }
}
