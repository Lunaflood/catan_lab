//! 交易（提案・対案・持ち時間）の検証。

use catan_core::action::*;
use catan_core::board::*;
use catan_core::game::*;
use catan_core::rng::Rng;

/// 初期配置を済ませ、P0 の手番でダイスを振った直後の状態にする。
fn ready(cfg: GameConfig) -> Game {
    let mut g = Game::with_config(4, 4242, cfg);
    let mut rng = Rng::with_stream(1, 1);
    let mut buf = Vec::new();
    while g.is_setup() {
        g.legal_actions_into(&mut buf);
        g.apply(buf[rng.below(buf.len() as u32) as usize]);
    }
    assert_eq!(g.turn_player, 0);
    g.rolled = true; // 産出の中身はここでは関係ないので直接立てる
    // 手札を決め打ちにする（銀行との辻褄も合わせる）
    for q in 0..4 {
        for i in 0..NUM_RESOURCES {
            g.bank[i] += g.players[q].hand[i];
            g.players[q].hand[i] = 0;
        }
    }
    g
}

fn set_hand(g: &mut Game, p: PlayerId, hand: Bundle) {
    for i in 0..NUM_RESOURCES {
        g.bank[i] += g.players[p as usize].hand[i];
        g.players[p as usize].hand[i] = 0;
        assert!(g.bank[i] >= hand[i], "銀行に足りない");
        g.bank[i] -= hand[i];
        g.players[p as usize].hand[i] = hand[i];
    }
}

const WOOD: usize = 0;
const BRICK: usize = 1;

#[test]
fn 対案が成立し資源が正しい向きに動く() {
    let mut g = ready(GameConfig::default());
    set_hand(&mut g, 0, [3, 0, 0, 0, 0]); // P0: 木3
    set_hand(&mut g, 1, [0, 2, 0, 0, 0]); // P1: 土2

    // P0「木1 出すので 土1 くれ」
    g.apply(Action::OfferTrade {
        give: [1, 0, 0, 0, 0],
        want: [0, 1, 0, 0, 0],
    });
    assert_eq!(g.prompt, Prompt::DecideTrade);
    assert_eq!(g.to_act, 1);

    // P1「土1 出すけど 木2 欲しい」— 対案
    let counter = Action::CounterOffer {
        give: [0, 1, 0, 0, 0],
        want: [2, 0, 0, 0, 0],
    };
    assert!(
        g.legal_actions().contains(&counter),
        "対案が合法手に出ていない"
    );
    g.apply(counter);

    // P2・P3 は拒否
    assert_eq!(g.to_act, 2);
    g.apply(Action::RejectTrade);
    assert_eq!(g.to_act, 3);
    g.apply(Action::RejectTrade);

    // 承諾はゼロでも対案があるので提案者の選択へ進む
    assert_eq!(g.prompt, Prompt::DecideAcceptees);
    assert_eq!(g.to_act, 0);
    let accept = Action::AcceptCounter { from: 1, alt: 0 };
    assert!(g.legal_actions().contains(&accept), "対案を受けられない");
    g.apply(accept);

    // P0: 木3-2=1, 土0+1=1 / P1: 木0+2=2, 土2-1=1
    assert_eq!(g.players[0].hand[WOOD], 1);
    assert_eq!(g.players[0].hand[BRICK], 1);
    assert_eq!(g.players[1].hand[WOOD], 2);
    assert_eq!(g.players[1].hand[BRICK], 1);
    assert_eq!(g.prompt, Prompt::PlayTurn);
    assert_eq!(g.to_act, 0);
    assert!(g.trade.is_none());
}

#[test]
fn 払えない対案は受けられない() {
    let mut g = ready(GameConfig::default());
    set_hand(&mut g, 0, [1, 0, 0, 0, 0]); // P0 は木1枚しか無い
    set_hand(&mut g, 1, [0, 2, 0, 0, 0]);

    g.apply(Action::OfferTrade {
        give: [1, 0, 0, 0, 0],
        want: [0, 1, 0, 0, 0],
    });
    // P1 が「木3 欲しい」と返す（P0 には払えない）
    g.apply(Action::CounterOffer {
        give: [0, 1, 0, 0, 0],
        want: [3, 0, 0, 0, 0],
    });
    g.apply(Action::RejectTrade);
    g.apply(Action::RejectTrade);

    assert_eq!(g.prompt, Prompt::DecideAcceptees);
    let acts = g.legal_actions();
    assert!(
        !acts.contains(&Action::AcceptCounter { from: 1, alt: 0 }),
        "払えない対案が合法手に出ている"
    );
    assert!(acts.contains(&Action::CancelTrade));
}

#[test]
fn 対案も持っていない資源では出せない() {
    let mut g = ready(GameConfig::default());
    set_hand(&mut g, 0, [2, 0, 0, 0, 0]);
    set_hand(&mut g, 1, [0, 0, 0, 0, 0]); // P1 は無一文
    g.apply(Action::OfferTrade {
        give: [1, 0, 0, 0, 0],
        want: [0, 1, 0, 0, 0],
    });
    let acts = g.legal_actions();
    assert!(
        !acts.iter().any(|a| matches!(a, Action::CounterOffer { .. })),
        "手札が無いのに対案が出ている"
    );
    assert!(acts.contains(&Action::RejectTrade));
    assert!(!acts.contains(&Action::AcceptTrade), "払えないのに承諾できている");
}

#[test]
fn 全員が単に拒否したら提案は流れる() {
    let mut g = ready(GameConfig::default());
    set_hand(&mut g, 0, [2, 0, 0, 0, 0]);
    g.apply(Action::OfferTrade {
        give: [1, 0, 0, 0, 0],
        want: [0, 1, 0, 0, 0],
    });
    for _ in 0..3 {
        g.apply(Action::RejectTrade);
    }
    assert_eq!(g.prompt, Prompt::PlayTurn);
    assert_eq!(g.to_act, 0);
    assert!(g.trade.is_none());
    assert_eq!(g.players[0].hand[WOOD], 2, "流れた提案で資源が動いた");
}

// =========================================================================
// 持ち時間
// =========================================================================

#[test]
fn 既定の持ち時間は2分() {
    let cfg = GameConfig::default();
    assert_eq!(cfg.turn_time_limit_ms, Some(120_000));
}

#[test]
fn 時間切れで交渉だけが止まり建設と手番終了は残る() {
    let cfg = GameConfig {
        turn_time_limit_ms: Some(1000),
        turn_clock: TurnClock::PerAction { ms: 400 },
        ..GameConfig::default()
    };
    let mut g = ready(cfg);
    // 道が建てられ、かつ 4:1 の海上交易もできる手札（4 枚無いと海上交易は成立しない）
    set_hand(&mut g, 0, [4, 2, 0, 0, 0]);

    assert!(g.negotiation_open());
    assert!(g
        .legal_actions()
        .iter()
        .any(|a| matches!(a, Action::OfferTrade { .. })));

    // 400ms × 3 = 1200ms で時間切れ
    for _ in 0..3 {
        g.apply(Action::EndTurn);
        // 手番が変わってしまうので、時計だけ進めたい今回は巻き戻す
        g.turn_player = 0;
        g.to_act = 0;
        g.rolled = true;
    }
    g.turn_elapsed_ms = 1200;

    assert!(!g.negotiation_open(), "時間切れになっていない");
    let acts = g.legal_actions();
    assert!(
        !acts.iter().any(|a| matches!(a, Action::OfferTrade { .. })),
        "時間切れなのに提案が出ている"
    );
    // 建設と手番終了は続けられる
    assert!(acts.contains(&Action::EndTurn));
    assert!(
        acts.iter().any(|a| matches!(a, Action::BuildRoad(_))),
        "時間切れで建設まで止まっている"
    );
    // 海上交易も残る（相手との交渉ではないため）
    assert!(acts.iter().any(|a| matches!(a, Action::MaritimeTrade { .. })));
}

#[test]
fn 実時間モードではtickで進む() {
    let cfg = GameConfig {
        turn_time_limit_ms: Some(5_000),
        turn_clock: TurnClock::Realtime,
        ..GameConfig::default()
    };
    let mut g = ready(cfg);
    set_hand(&mut g, 0, [2, 0, 0, 0, 0]);

    // 行動しても勝手には進まない
    g.apply(Action::OfferTrade {
        give: [1, 0, 0, 0, 0],
        want: [0, 1, 0, 0, 0],
    });
    assert_eq!(g.turn_elapsed_ms, 0, "実時間モードで時計が勝手に進んだ");
    assert_eq!(g.turn_time_left_ms(), Some(5_000));

    g.tick(4_000);
    assert_eq!(g.turn_time_left_ms(), Some(1_000));
    assert!(g.negotiation_open());
    g.tick(1_500);
    assert_eq!(g.turn_time_left_ms(), Some(0));
    assert!(!g.negotiation_open());
}

#[test]
fn 無制限にもできる() {
    let cfg = GameConfig {
        turn_time_limit_ms: None,
        turn_clock: TurnClock::PerAction { ms: 10_000 },
        ..GameConfig::default()
    };
    let mut g = ready(cfg);
    set_hand(&mut g, 0, [2, 0, 0, 0, 0]);
    g.turn_elapsed_ms = 10_000_000;
    assert!(g.negotiation_open());
    assert_eq!(g.turn_time_left_ms(), None);
    assert!(g
        .legal_actions()
        .iter()
        .any(|a| matches!(a, Action::OfferTrade { .. })));
}

#[test]
fn 手番が変わると持ち時間はリセットされる() {
    let mut g = ready(GameConfig::default());
    g.turn_elapsed_ms = 100_000;
    g.apply(Action::EndTurn);
    assert_eq!(g.turn_elapsed_ms, 0);
    assert_eq!(g.turn_player, 1);
}

// =========================================================================
// 断られた提案
// =========================================================================

#[test]
fn 一度断られた条件は同じ手番で出し直せない() {
    let mut g = ready(GameConfig::default());
    set_hand(&mut g, 0, [3, 0, 0, 0, 0]);

    let offer = Action::OfferTrade {
        give: [1, 0, 0, 0, 0],
        want: [0, 1, 0, 0, 0],
    };
    assert!(g.legal_actions().contains(&offer));

    g.apply(offer);
    for _ in 0..3 {
        g.apply(Action::RejectTrade);
    }
    assert_eq!(g.prompt, Prompt::PlayTurn);

    assert!(g.offer_was_rejected(&[1, 0, 0, 0, 0], &[0, 1, 0, 0, 0]));
    assert!(
        !g.legal_actions().contains(&offer),
        "断られた条件がまた合法手に出ている（これで手番が終わらなくなる）"
    );
    // 別の条件は出せる
    assert!(g.legal_actions().iter().any(|a| matches!(
        a,
        Action::OfferTrade { want, .. } if want[2] > 0 || want[3] > 0 || want[4] > 0
    )));
}

#[test]
fn 取り下げた条件も出し直せない() {
    let mut g = ready(GameConfig::default());
    set_hand(&mut g, 0, [3, 0, 0, 0, 0]);
    set_hand(&mut g, 1, [0, 2, 0, 0, 0]);
    let offer = Action::OfferTrade {
        give: [1, 0, 0, 0, 0],
        want: [0, 1, 0, 0, 0],
    };
    g.apply(offer);
    g.apply(Action::AcceptTrade); // P1 は受ける
    g.apply(Action::RejectTrade);
    g.apply(Action::RejectTrade);
    assert_eq!(g.prompt, Prompt::DecideAcceptees);
    g.apply(Action::CancelTrade); // 提案者が下げる
    assert!(!g.legal_actions().contains(&offer));
}

#[test]
fn 手番が変われば同じ条件をまた出せる() {
    let mut g = ready(GameConfig::default());
    set_hand(&mut g, 0, [3, 0, 0, 0, 0]);
    let offer = Action::OfferTrade {
        give: [1, 0, 0, 0, 0],
        want: [0, 1, 0, 0, 0],
    };
    g.apply(offer);
    for _ in 0..3 {
        g.apply(Action::RejectTrade);
    }
    assert_ne!(g.rejected_offers, [0; 2]);
    g.apply(Action::EndTurn);
    assert_eq!(g.rejected_offers, [0; 2], "手番が変わってもリセットされていない");
}

#[test]
fn 提案の番号づけは生成する形を全部拾う() {
    use catan_core::game::canonical_offer_index;
    // 1:1 / 2:1 / 1:2 は番号がつく
    assert!(canonical_offer_index(&[1, 0, 0, 0, 0], &[0, 1, 0, 0, 0]).is_some());
    assert!(canonical_offer_index(&[2, 0, 0, 0, 0], &[0, 1, 0, 0, 0]).is_some());
    assert!(canonical_offer_index(&[1, 0, 0, 0, 0], &[0, 2, 0, 0, 0]).is_some());
    // 種類も枚数も違えば別の番号
    let a = canonical_offer_index(&[1, 0, 0, 0, 0], &[0, 1, 0, 0, 0]).unwrap();
    let b = canonical_offer_index(&[2, 0, 0, 0, 0], &[0, 1, 0, 0, 0]).unwrap();
    let c = canonical_offer_index(&[1, 0, 0, 0, 0], &[0, 0, 1, 0, 0]).unwrap();
    assert!(a != b && a != c && b != c);
    // 2 種類の束にも番号がつく（生成する形なので、断られたら覚えておく必要がある）
    assert!(canonical_offer_index(&[1, 1, 0, 0, 0], &[0, 0, 1, 0, 0]).is_some());
    assert!(canonical_offer_index(&[1, 0, 0, 0, 0], &[0, 1, 1, 0, 0]).is_some());
    // 多めに積む形・値切る形も生成するので番号がつく
    // （ここが無いと「人間だけが出せる条件」が残る）
    assert!(canonical_offer_index(&[3, 0, 0, 0, 0], &[0, 1, 0, 0, 0]).is_some());
    assert!(canonical_offer_index(&[1, 0, 0, 0, 0], &[0, 3, 0, 0, 0]).is_some());
    assert!(canonical_offer_index(&[2, 0, 0, 0, 0], &[0, 2, 0, 0, 0]).is_some());
    // 生成しない形（3 種類以上・4 枚以上）は対象外。覚えないだけで、指すことはできる
    assert!(canonical_offer_index(&[1, 1, 1, 0, 0], &[0, 0, 0, 1, 0]).is_none());
    assert!(canonical_offer_index(&[4, 0, 0, 0, 0], &[0, 1, 0, 0, 0]).is_none());

    // 全部の形の番号が重ならず、[u128; 2] の 256 ビットに収まること
    let mut seen = std::collections::BTreeSet::new();
    let mut max = 0u8;
    for gi in 0..5usize {
        for wi in 0..5usize {
            if gi == wi {
                continue;
            }
            for (gn, wn) in [(1u8, 1u8), (2, 1), (1, 2), (3, 1), (1, 3), (2, 2)] {
                let (mut g, mut w) = ([0u8; 5], [0u8; 5]);
                g[gi] = gn;
                w[wi] = wn;
                let i = canonical_offer_index(&g, &w).expect("番号がつくはず");
                assert!(seen.insert(i), "番号が重なった: {i}");
                max = max.max(i);
            }
        }
    }
    assert!(max < 180, "想定した範囲 (180) に収まること: {max}");
}

#[test]
fn seedごとに最初の出目は変わる() {
    // 「何試合やっても最初に 7 が出る」という報告の切り分け。
    // 同じ seed なら同じ試合になるのは設計どおり（再現性のため）。
    // seed を変えた時にちゃんとばらけるかを見る。
    let mut first = Vec::new();
    for seed in 0..40u64 {
        let mut g = Game::new(4, seed);
        let mut rng = Rng::with_stream(seed, 77);
        let mut buf = Vec::new();
        while g.is_setup() {
            g.legal_actions_into(&mut buf);
            g.apply(buf[rng.below(buf.len() as u32) as usize]);
        }
        let rec = g.apply(Action::Roll);
        let catan_core::action::Outcome::Dice(a, b) = rec.result else {
            panic!("ダイスが返らない")
        };
        first.push(a + b);
    }
    eprintln!("seed 0..40 の最初の出目: {first:?}");
    let sevens = first.iter().filter(|&&v| v == 7).count();
    let uniq: std::collections::BTreeSet<_> = first.iter().copied().collect();
    eprintln!("うち 7 は {sevens} 回 / 出た目の種類 {}", uniq.len());
    assert!(uniq.len() >= 6, "最初の出目がばらけていない: {uniq:?}");
    assert!(sevens <= 14, "7 に偏りすぎ: {sevens}/40");
}

#[test]
fn 提案の番号づけは120通りが全部別々になる() {
    use catan_core::board::{RESOURCES, NUM_RESOURCES};
    use std::collections::HashSet;

    let mut seen: HashSet<u8> = HashSet::new();
    let one = |r: usize, n: u8| {
        let mut b = catan_core::action::EMPTY;
        b[r] = n;
        b
    };
    let two = |a: usize, b: usize| {
        let mut x = catan_core::action::EMPTY;
        x[a] = 1;
        x[b] = 1;
        x
    };
    let mut count = 0;
    for g in 0..NUM_RESOURCES {
        for w in 0..NUM_RESOURCES {
            if g == w {
                continue;
            }
            for (gn, wn) in [(1u8, 1u8), (2, 1), (1, 2)] {
                let i = canonical_offer_index(&one(g, gn), &one(w, wn)).expect("番号が付かない");
                assert!(seen.insert(i), "番号が衝突した: {i}");
                count += 1;
            }
        }
    }
    for a in 0..NUM_RESOURCES {
        for b in (a + 1)..NUM_RESOURCES {
            for w in 0..NUM_RESOURCES {
                if w == a || w == b {
                    continue;
                }
                let i = canonical_offer_index(&two(a, b), &one(w, 1)).expect("番号が付かない");
                assert!(seen.insert(i), "番号が衝突した: {i}");
                count += 1;
            }
            for g in 0..NUM_RESOURCES {
                if g == a || g == b {
                    continue;
                }
                let i = canonical_offer_index(&one(g, 1), &two(a, b)).expect("番号が付かない");
                assert!(seen.insert(i), "番号が衝突した: {i}");
                count += 1;
            }
        }
    }
    assert_eq!(count, 120, "生成する提案の形は 120 通りのはず");
    assert!(seen.iter().all(|&i| i < 128), "u128 のビットに収まらない");
    let _ = RESOURCES;
}

#[test]
fn 広い生成では複数種類の束が出る() {
    let mut g = ready(GameConfig::default());
    g.players[0].hand = [2, 2, 2, 2, 2];
    let acts = g.legal_actions();
    let multi = acts.iter().any(|a| match a {
        Action::OfferTrade { give, want } => {
            give.iter().filter(|&&n| n > 0).count() > 1 || want.iter().filter(|&&n| n > 0).count() > 1
        }
        _ => false,
    });
    assert!(multi, "複数種類の束の提案が生成されていない");
}

#[test]
fn 狭い生成では単一資源だけになる() {
    let cfg = GameConfig { wide_offers: false, ..GameConfig::default() };
    let mut g = ready(cfg);
    g.players[0].hand = [2, 2, 2, 2, 2];
    for a in g.legal_actions() {
        if let Action::OfferTrade { give, want } = a {
            assert_eq!(give.iter().filter(|&&n| n > 0).count(), 1);
            assert_eq!(want.iter().filter(|&&n| n > 0).count(), 1);
        }
    }
}

#[test]
fn 同種を含む交換は生成されない() {
    let mut g = ready(GameConfig::default());
    g.players[0].hand = [2, 2, 2, 2, 2];
    for a in g.legal_actions() {
        if let Action::OfferTrade { give, want } = a {
            for i in 0..NUM_RESOURCES {
                assert!(give[i] == 0 || want[i] == 0, "同種を両側に置く提案が出た");
            }
        }
    }
}

#[test]
fn 盤面の種と出目の種は独立している() {
    let cfg = GameConfig::default();
    let tiles = |g: &Game| (g.board.tile_resource, g.board.tile_number);
    let first_roll = |mut g: Game| {
        let mut buf = Vec::new();
        let mut rng = Rng::with_stream(9, 9);
        while g.is_setup() {
            g.legal_actions_into(&mut buf);
            g.apply(buf[rng.below(buf.len() as u32) as usize]);
        }
        match g.apply(Action::Roll).result {
            Outcome::Dice(a, b) => a + b,
            _ => unreachable!(),
        }
    };

    // 盤の種だけ変える → 盤は変わる
    let a = Game::with_seeds(4, 1, 100, cfg);
    let b = Game::with_seeds(4, 2, 100, cfg);
    assert_ne!(tiles(&a), tiles(&b), "盤の種を変えても盤が同じ");

    // 出目の種だけ変える → 盤は同じ
    let c = Game::with_seeds(4, 1, 200, cfg);
    assert_eq!(tiles(&a), tiles(&c), "出目の種で盤まで変わっている");

    // 同じ盤で出目だけ違う組を探す（1 回の出目は 11 通りなので何組か見る）
    let mut differs = false;
    for s in 200..240u64 {
        if first_roll(Game::with_seeds(4, 1, s, cfg)) != first_roll(Game::with_seeds(4, 1, 100, cfg)) {
            differs = true;
            break;
        }
    }
    assert!(differs, "出目の種を変えても出目が変わらない");
}

#[test]
fn 対案は候補を並べて提案者に選ばせられる() {
    // 「木か土ならいいよ」＝ 成立してよい条件を 2 本並べる
    let mut g = ready(GameConfig::default());
    g.players[0].hand = [0, 0, 0, 3, 0]; // 提案者は小麦しか持たない
    g.players[1].hand = [2, 2, 0, 0, 0]; // 対案を返す側は木と土
    g.apply(Action::OfferTrade { give: bundle_of(Resource::Wheat, 1), want: bundle_of(Resource::Sheep, 1) });

    // P1 が候補を 2 本積んで返答する
    assert_eq!(g.to_act, 1);
    g.apply(Action::CounterAlt {
        give: bundle_of(Resource::Wood, 1),
        want: bundle_of(Resource::Wheat, 1),
    });
    assert_eq!(g.to_act, 1, "候補を積んだだけでは手番は動かない");
    g.apply(Action::CounterOffer {
        give: bundle_of(Resource::Brick, 1),
        want: bundle_of(Resource::Wheat, 1),
    });
    assert_ne!(g.to_act, 1, "対案を返したら次の人へ回る");

    // 残りは断る
    while g.prompt == Prompt::DecideTrade {
        g.apply(Action::RejectTrade);
    }
    assert_eq!(g.prompt, Prompt::DecideAcceptees);

    // 提案者には候補が 2 本とも見えている
    let acts = g.legal_actions();
    let alts: Vec<u8> = acts
        .iter()
        .filter_map(|a| match a {
            Action::AcceptCounter { from: 1, alt } => Some(*alt),
            _ => None,
        })
        .collect();
    assert_eq!(alts.len(), 2, "候補が 2 本とも選べるはず: {acts:?}");

    // 土の方（候補 1）を選ぶ
    let before_wheat = g.players[0].hand[Resource::Wheat.idx()];
    g.apply(Action::AcceptCounter { from: 1, alt: 1 });
    assert_eq!(g.players[0].hand[Resource::Brick.idx()], 1, "選んだ方の資源が来る");
    assert_eq!(g.players[0].hand[Resource::Wood.idx()], 0, "選ばなかった方は来ない");
    assert_eq!(g.players[0].hand[Resource::Wheat.idx()], before_wheat - 1);
}

#[test]
fn 対案の候補は同じ条件を重ねない() {
    let mut g = ready(GameConfig::default());
    g.players[0].hand = [0, 0, 0, 3, 0];
    g.players[1].hand = [2, 2, 0, 0, 0];
    g.apply(Action::OfferTrade { give: bundle_of(Resource::Wheat, 1), want: bundle_of(Resource::Sheep, 1) });
    let same = Action::CounterAlt {
        give: bundle_of(Resource::Wood, 1),
        want: bundle_of(Resource::Wheat, 1),
    };
    g.apply(same);
    g.apply(same);
    g.apply(same);
    let t = g.trade.expect("交易中");
    assert_eq!(t.counters[1].iter().filter(|c| c.is_some()).count(), 1);
}

#[test]
fn 出来事ごとの流れは互いに影響しない() {
    let cfg = GameConfig::default();
    // 発展カードの種だけ変えても、サイコロの並びは 1 つも変わらない
    let rolls = |dev_seed: u64| {
        let mut g = Game::with_streams(4, 1, 42, dev_seed, 7, cfg);
        let mut rng = Rng::with_stream(3, 3);
        let mut buf = Vec::new();
        while g.is_setup() {
            g.legal_actions_into(&mut buf);
            g.apply(buf[rng.below(buf.len() as u32) as usize]);
        }
        let mut out = Vec::new();
        for _ in 0..12 {
            if let Outcome::Dice(a, b) = g.apply(Action::Roll).result {
                out.push(a + b);
            }
            // 次の手番へ（廃棄や盗賊が挟まっても最後は EndTurn に着く）
            for _ in 0..200 {
                if g.prompt == Prompt::PlayTurn && g.rolled {
                    g.apply(Action::EndTurn);
                    break;
                }
                g.legal_actions_into(&mut buf);
                g.apply(buf[rng.below(buf.len() as u32) as usize]);
            }
        }
        out
    };
    assert_eq!(rolls(100), rolls(999), "発展カードの種でサイコロが変わっている");

    // サイコロの種を変えれば並びは変わる
    let a = Game::with_streams(4, 1, 1, 5, 5, cfg).rng_dice.clone().dice();
    let mut differs = false;
    for s in 2..40u64 {
        if Game::with_streams(4, 1, s, 5, 5, cfg).rng_dice.clone().dice() != a {
            differs = true;
            break;
        }
    }
    assert!(differs, "サイコロの種を変えても出目が変わらない");
}

#[test]
fn 積んだ対案の候補は取り消せる() {
    let mut g = ready(GameConfig::default());
    g.players[0].hand = [0, 0, 0, 3, 0];
    g.players[1].hand = [2, 2, 2, 0, 0];
    g.apply(Action::OfferTrade {
        give: bundle_of(Resource::Wheat, 1),
        want: bundle_of(Resource::Sheep, 1),
    });
    assert_eq!(g.to_act, 1);

    let alt = |r: Resource| Action::CounterAlt {
        give: bundle_of(r, 1),
        want: bundle_of(Resource::Wheat, 1),
    };
    g.apply(alt(Resource::Wood));
    g.apply(alt(Resource::Brick));
    g.apply(alt(Resource::Sheep));
    let n = |g: &Game| g.trade.unwrap().counters[1].iter().filter(|c| c.is_some()).count();
    assert_eq!(n(&g), 3);

    // 真ん中を消すと、後ろが前へ詰まる（UI の番号とずれないように）
    g.apply(Action::CounterAltRemove(1));
    assert_eq!(n(&g), 2);
    let left = g.trade.unwrap().counters[1];
    assert_eq!(left[0].unwrap().0, bundle_of(Resource::Wood, 1));
    assert_eq!(left[1].unwrap().0, bundle_of(Resource::Sheep, 1));
    assert!(left[2].is_none(), "空きが詰まっていない");

    // 取り消しても返答は終わっていない
    assert_eq!(g.to_act, 1);
    assert_eq!(g.prompt, Prompt::DecideTrade);

    // 消した枠は使い回せる
    g.apply(alt(Resource::Brick));
    assert_eq!(n(&g), 3);
}

#[test]
#[should_panic(expected = "持ち時間")]
fn 時計が切れた後に古い提案を指すと弾かれる() {
    // ブラウザで実際に起きた落ち方の再現。
    // 手の一覧を作った後に時計が進んで交渉が閉じると、
    // その一覧に入っていた提案はもう指せない（`apply` が弾く）。
    // UI 側は時計を進めたら一覧を作り直さなければならない。
    let cfg = GameConfig {
        turn_clock: TurnClock::Realtime,
        turn_time_limit_ms: Some(1000),
        ..GameConfig::default()
    };
    let mut g = ready(cfg);
    g.players[0].hand = [2, 2, 2, 2, 2];

    // まだ開いているので提案が並ぶ
    let acts = g.legal_actions();
    let offer = acts
        .iter()
        .find(|a| matches!(a, Action::OfferTrade { .. }))
        .copied()
        .expect("提案が生成されていない");

    // ここで時計が切れる
    g.tick(2000);
    assert!(!g.negotiation_open());

    // 作り直していない古い手を指すと弾かれる
    g.apply(offer);
}

#[test]
fn 時計が切れたら提案は一覧から消える() {
    let cfg = GameConfig {
        turn_clock: TurnClock::Realtime,
        turn_time_limit_ms: Some(1000),
        ..GameConfig::default()
    };
    let mut g = ready(cfg);
    g.players[0].hand = [2, 2, 2, 2, 2];
    assert!(g.legal_actions().iter().any(|a| matches!(a, Action::OfferTrade { .. })));

    g.tick(2000);
    let after = g.legal_actions();
    assert!(
        !after.iter().any(|a| matches!(a, Action::OfferTrade { .. })),
        "時間切れなのに提案が残っている"
    );
    // 建設と手番終了は残る
    assert!(after.contains(&Action::EndTurn));
}

#[test]
fn 積んだ候補だけで返答を確定できる() {
    // UI は「候補を積む → 返す」の 1 本道。
    // 確定は最後に積んだ 1 本を CounterOffer として指すので、
    // 返答の中身は **積んだ候補ちょうど** になる（作りかけの条件が紛れ込まない）。
    let mut g = ready(GameConfig::default());
    g.players[0].hand = [0, 0, 0, 3, 0];
    g.players[1].hand = [2, 2, 2, 0, 0];
    g.apply(Action::OfferTrade {
        give: bundle_of(Resource::Wheat, 1),
        want: bundle_of(Resource::Sheep, 1),
    });

    let alt = |r: Resource| (bundle_of(r, 1), bundle_of(Resource::Wheat, 1));
    let (g1, w1) = alt(Resource::Wood);
    let (g2, w2) = alt(Resource::Brick);
    g.apply(Action::CounterAlt { give: g1, want: w1 });
    g.apply(Action::CounterAlt { give: g2, want: w2 });

    // 最後の 1 本で確定する（重複は積まれない）
    g.apply(Action::CounterOffer { give: g2, want: w2 });
    let t = g.trade.expect("交易中");
    let n = t.counters[1].iter().filter(|c| c.is_some()).count();
    assert_eq!(n, 2, "確定で候補が増えている: {:?}", t.counters[1]);
    assert_eq!(t.counters[1][0].unwrap().0, g1);
    assert_eq!(t.counters[1][1].unwrap().0, g2);
    assert_ne!(g.to_act, 1, "返答が終わっていない");
}

/// CPU が返す対案は、必ず**提案者が欲しがっていた資源**を出す物でなければならない。
///
/// ここを絞らないと「木が欲しい」と言った相手に「鉄をやるから羊をくれ」と
/// まったく噛み合わない対案が返り、提案者は断るしかない札で候補欄が埋まる。
#[test]
fn 生成される対案は提案者の欲しい資源を必ず含む() {
    let mut g = ready(GameConfig::default());
    set_hand(&mut g, 0, [3, 0, 0, 0, 0]); // P0: 木3
    set_hand(&mut g, 1, [0, 2, 2, 2, 2]); // P1: 木以外を全部持っている

    // P0「木1 出すので 土1 くれ」＝ 欲しいのは土
    g.apply(Action::OfferTrade {
        give: [1, 0, 0, 0, 0],
        want: [0, 1, 0, 0, 0],
    });
    assert_eq!(g.prompt, Prompt::DecideTrade);

    let mut seen = 0;
    for a in g.legal_actions() {
        if let Action::CounterOffer { give, .. } = a {
            seen += 1;
            assert!(
                give[BRICK] > 0,
                "提案者が欲しがっていない資源だけの対案が出た: give={give:?}"
            );
        }
    }
    assert!(seen > 0, "対案の候補が 1 つも作られていない");
}

/// 提案側（対案ではない方）は今まで通り自由に作れる。上の制限が漏れていないこと。
#[test]
fn 提案の生成は絞られていない() {
    let mut g = ready(GameConfig::default());
    set_hand(&mut g, 0, [2, 2, 0, 0, 0]);
    let kinds: Vec<_> = g
        .legal_actions()
        .into_iter()
        .filter_map(|a| match a {
            Action::OfferTrade { give, .. } => Some(give),
            _ => None,
        })
        .collect();
    assert!(kinds.iter().any(|g| g[WOOD] > 0), "木を出す提案が無い");
    assert!(kinds.iter().any(|g| g[BRICK] > 0), "土を出す提案が無い");
}

/// 1 手番に持ちかけられる回数には上限がある。
/// 区切らないと CPU が断られるたびに別の条件を作り、人は答えるだけで手番が終わる。
#[test]
fn 提案は一手番の回数で打ち切られる() {
    let mut cfg = GameConfig::default();
    cfg.max_offers_per_turn = 2;
    let mut g = ready(cfg);
    set_hand(&mut g, 0, [3, 3, 0, 0, 0]);
    set_hand(&mut g, 1, [0, 0, 3, 0, 0]);

    let has_offer = |g: &Game| g.legal_actions().iter().any(|a| matches!(a, Action::OfferTrade { .. }));
    assert!(has_offer(&g), "はじめは提案できる");

    // 1 回目
    g.apply(Action::OfferTrade { give: [1, 0, 0, 0, 0], want: [0, 0, 1, 0, 0] });
    while g.prompt == Prompt::DecideTrade {
        g.apply(Action::RejectTrade);
    }
    if g.prompt == Prompt::DecideAcceptees {
        g.apply(Action::CancelTrade);
    }
    assert!(has_offer(&g), "2 回目はまだ出せる");

    // 2 回目
    g.apply(Action::OfferTrade { give: [0, 1, 0, 0, 0], want: [0, 0, 1, 0, 0] });
    while g.prompt == Prompt::DecideTrade {
        g.apply(Action::RejectTrade);
    }
    if g.prompt == Prompt::DecideAcceptees {
        g.apply(Action::CancelTrade);
    }
    assert!(!has_offer(&g), "上限に達したら、もう提案は作られない");

    // 手番が変われば元に戻る
    g.apply(Action::EndTurn);
    while g.turn_player != 0 {
        let acts = g.legal_actions();
        let end = acts.iter().find(|a| matches!(a, Action::EndTurn)).copied();
        match end {
            Some(a) => {
                g.apply(a);
            }
            None => {
                g.apply(acts[0]);
            }
        }
    }
    assert_eq!(g.offers_this_turn, 0, "手番が一周したら回数は戻る");
}

/// 「?」の札 ── 片側を空にした「相談」の提案。
/// 「木を出すから何かくれ」／「木が欲しい、代わりは何がいい？」の形。
/// そのまま承諾はできず、対案でしか答えられない。
#[test]
fn 片側が空の相談は対案でしか答えられない() {
    let mut g = ready(GameConfig::default());
    set_hand(&mut g, 0, [3, 0, 0, 0, 0]); // P0: 木3
    set_hand(&mut g, 1, [0, 2, 0, 0, 0]); // P1: 土2

    // 「木1 を出す。代わりは何でもいい」
    g.apply(Action::OfferTrade {
        give: [1, 0, 0, 0, 0],
        want: [0, 0, 0, 0, 0],
    });
    assert_eq!(g.prompt, Prompt::DecideTrade);
    let acts = g.legal_actions();
    assert!(
        !acts.iter().any(|a| matches!(a, Action::AcceptTrade)),
        "相談をそのまま受けられてはいけない（ただの贈与になる）"
    );
    assert!(acts.iter().any(|a| matches!(a, Action::RejectTrade)), "断ることはできる");

    // 対案で中身を埋めれば成立する
    g.apply(Action::CounterOffer {
        give: [0, 1, 0, 0, 0],
        want: [1, 0, 0, 0, 0],
    });
    while g.prompt == Prompt::DecideTrade {
        g.apply(Action::RejectTrade);
    }
    assert_eq!(g.prompt, Prompt::DecideAcceptees);
    let pick = g
        .legal_actions()
        .into_iter()
        .find(|a| matches!(a, Action::AcceptCounter { .. }))
        .expect("対案を受ける手がある");
    g.apply(pick);
    assert_eq!(g.players[0].hand[WOOD], 2, "木を 1 枚出した");
    assert_eq!(g.players[0].hand[BRICK], 1, "土を 1 枚もらった");
}

/// 逆向きの相談。「木が欲しい。そちらは何が要る？」
#[test]
fn 欲しい物だけの相談も出せる() {
    let mut g = ready(GameConfig::default());
    set_hand(&mut g, 0, [0, 3, 0, 0, 0]);
    set_hand(&mut g, 1, [2, 0, 0, 0, 0]);
    g.apply(Action::OfferTrade {
        give: [0, 0, 0, 0, 0],
        want: [1, 0, 0, 0, 0],
    });
    assert_eq!(g.prompt, Prompt::DecideTrade);
    assert!(!g.legal_actions().iter().any(|a| matches!(a, Action::AcceptTrade)));
}
