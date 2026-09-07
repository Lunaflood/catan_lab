//! 交易 AI の振る舞いを固定するテスト。
//!
//! 勝率はハーネスで測る。ここで確かめるのは「なぜその判断になるか」の方。

use catan_ai::eval::{evaluate, threats, EvalWeights};
use catan_ai::trade::{eval_delta, preview};
use catan_core::action::{Action, Bundle};
use catan_core::board::{BuildingKind, PlayerId, NUM_RESOURCES};
use catan_core::game::{Game, GameConfig};
use catan_core::rng::Rng;
use catan_core::topology::{NodeId, Topology, NUM_NODES};

const WOOD: usize = 0;
const BRICK: usize = 1;
const SHEEP: usize = 2;
const WHEAT: usize = 3;
const ORE: usize = 4;

/// 初期配置を済ませ、P0 の手番でダイスを振った直後にする。
fn ready() -> Game {
    let mut g = Game::with_config(4, 4242, GameConfig::default());
    let mut rng = Rng::with_stream(1, 1);
    let mut buf = Vec::new();
    while g.is_setup() {
        g.legal_actions_into(&mut buf);
        g.apply(buf[rng.below(buf.len() as u32) as usize]);
    }
    g.rolled = true;
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
        g.bank[i] -= hand[i];
        g.players[p as usize].hand[i] = hand[i];
    }
}

/// `p` に勝利点を持たせる（開拓地を都市に変えて公開VPを上げる）
fn make_leader(g: &mut Game, p: PlayerId) {
    let topo = Topology::get();
    for n in 0..NUM_NODES {
        if let Some(b) = g.board.building[n] {
            if b.owner == p && b.kind == BuildingKind::Settlement {
                g.board.building[n] = Some(catan_core::board::Building {
                    owner: p,
                    kind: BuildingKind::City,
                });
                g.players[p as usize].cities_left -= 1;
                g.players[p as usize].settlements_left += 1;
            }
        }
    }
    let _ = topo;
}

#[test]
fn 建設を成立させる交換は正の評価になる() {
    let w = EvalWeights::default();
    let mut g = ready();
    // P0: 木2 土1 羊1 → 麦が 1 枚来れば開拓地が建つ
    set_hand(&mut g, 0, [2, 1, 1, 0, 0]);
    // P1: 麦が余っていて木が無い → 木が 1 枚来れば向こうも開拓地が建つ
    set_hand(&mut g, 1, [0, 1, 1, 3, 0]);
    set_hand(&mut g, 2, [0, 0, 0, 0, 0]);
    set_hand(&mut g, 3, [0, 0, 0, 0, 0]);

    let a = Action::OfferTrade {
        give: [1, 0, 0, 0, 0],
        want: [0, 0, 0, 1, 0],
    };
    let d = eval_delta(&g, 0, a, &w, &threats(&g, &w)).expect("双方が得なので成立するはず");
    assert!(d > 0.0, "建設を成立させる交換が正に評価されない: {d}");
}

#[test]
fn 建設が成立する交換は微差の交換より桁違いに高く出る() {
    // 「手札の噛み合い」という連続量だけで評価していた頃は、どちらにも大差ない
    // 1:1 交換が常にわずかに正に見え、1 試合で 1,163 回交易して対局が終わらなくなった。
    // 対策は連続量を消すことではなく、「実際に払えるようになる」段差を上に積むこと。
    // 微差の交換もわずかに正のままだが、建設や他の手に argmax で負けるようになる。
    let w = EvalWeights::default();

    // (a) 何も建たない微差の交換
    let mut g = ready();
    set_hand(&mut g, 0, [4, 0, 0, 0, 0]);
    set_hand(&mut g, 1, [0, 4, 0, 0, 0]);
    set_hand(&mut g, 2, [0, 0, 0, 0, 0]);
    set_hand(&mut g, 3, [0, 0, 0, 0, 0]);
    let marginal = eval_delta(
        &g,
        0,
        Action::OfferTrade {
            give: [1, 0, 0, 0, 0],
            want: [0, 1, 0, 0, 0],
        },
        &w,
        &threats(&g, &w),
    )
    .unwrap_or(0.0);

    // (b) 開拓地が建つようになる交換
    let mut g = ready();
    set_hand(&mut g, 0, [2, 1, 1, 0, 0]);
    set_hand(&mut g, 1, [0, 1, 1, 3, 0]);
    set_hand(&mut g, 2, [0, 0, 0, 0, 0]);
    set_hand(&mut g, 3, [0, 0, 0, 0, 0]);
    let decisive = eval_delta(
        &g,
        0,
        Action::OfferTrade {
            give: [1, 0, 0, 0, 0],
            want: [0, 0, 0, 1, 0],
        },
        &w,
        &threats(&g, &w),
    )
    .expect("双方が得なので成立するはず");

    assert!(
        decisive > marginal * 5.0,
        "建設が成立する交換 {decisive} が微差の交換 {marginal} と大差ない"
    );
}

#[test]
fn 相手が損する交換は成立しないと判定される() {
    let w = EvalWeights::default();
    let mut g = ready();
    // P0 が「木1 で 鉄1」を要求。相手は都市に必要な鉄を手放す
    set_hand(&mut g, 0, [3, 0, 0, 0, 0]);
    set_hand(&mut g, 1, [0, 0, 0, 2, 3]); // 都市が建つ直前
    set_hand(&mut g, 2, [0, 0, 0, 2, 3]);
    set_hand(&mut g, 3, [0, 0, 0, 2, 3]);

    let a = Action::OfferTrade {
        give: [1, 0, 0, 0, 0],
        want: [0, 0, 0, 0, 1],
    };
    assert!(
        eval_delta(&g, 0, a, &w, &threats(&g, &w)).is_none(),
        "都市が建つ直前の相手から鉄を抜ける前提になっている"
    );
}

#[test]
fn リーダーを利する交換は避けられる() {
    let w = EvalWeights::default();
    let mut g = ready();
    set_hand(&mut g, 0, [3, 0, 0, 0, 0]);
    // P1 と P2 が同じ手札。P1 だけ勝利点が高い
    set_hand(&mut g, 1, [0, 2, 0, 0, 0]);
    set_hand(&mut g, 2, [0, 2, 0, 0, 0]);
    set_hand(&mut g, 3, [0, 0, 0, 0, 0]);
    make_leader(&mut g, 1);
    assert!(g.public_vp(1) > g.public_vp(2), "リーダーを作れていない");

    let a = Action::OfferTrade {
        give: [1, 0, 0, 0, 0],
        want: [0, 1, 0, 0, 0],
    };
    let sim = preview(&g, 0, a, &w).expect("成立するはず");
    // 同じ条件で払える相手が 2 人いるなら、勝利点の低い方を選ぶ
    assert_eq!(
        sim.players[2].hand[WOOD], 1,
        "リーダー(P1)ではなく P2 と交換するはず"
    );
    assert_eq!(sim.players[1].hand[BRICK], 2, "リーダーの手札が動いている");
}

#[test]
fn 交易は資源の総数を変えない() {
    let w = EvalWeights::default();
    let mut g = ready();
    set_hand(&mut g, 0, [4, 0, 0, 0, 0]);
    set_hand(&mut g, 1, [0, 3, 1, 1, 0]);
    let a = Action::OfferTrade {
        give: [2, 0, 0, 0, 0],
        want: [0, 1, 0, 0, 0],
    };
    let before: Vec<u32> = (0..NUM_RESOURCES)
        .map(|i| (0..4).map(|q| g.players[q].hand[i] as u32).sum::<u32>() + g.bank[i] as u32)
        .collect();
    let sim = preview(&g, 0, a, &w).expect("成立するはず");
    let after: Vec<u32> = (0..NUM_RESOURCES)
        .map(|i| {
            (0..4).map(|q| sim.players[q].hand[i] as u32).sum::<u32>() + sim.bank[i] as u32
        })
        .collect();
    assert_eq!(before, after, "交易で資源が湧いた/消えた");
}

#[test]
fn 対案は提案者が払えなければ出さない() {
    let w = EvalWeights::default();
    let mut g = ready();
    set_hand(&mut g, 0, [1, 0, 0, 0, 0]); // 木1枚しか無い
    set_hand(&mut g, 1, [0, 2, 0, 0, 0]);
    g.apply(Action::OfferTrade {
        give: [1, 0, 0, 0, 0],
        want: [0, 1, 0, 0, 0],
    });
    // P1 が「木3 欲しい」と返しても P0 は払えない
    let a = Action::CounterOffer {
        give: [0, 1, 0, 0, 0],
        want: [3, 0, 0, 0, 0],
    };
    assert!(eval_delta(&g, 1, a, &w, &threats(&g, &w)).is_none());
}

#[test]
fn 交易の差分評価は局面を作り直した結果と一致する() {
    // 速度のために手札の項だけ差し引きしている。全部計算し直した値とズレていないこと。
    let w = EvalWeights::default();
    let mut rng = Rng::new(99);
    let mut checked = 0;
    for seed in 0..40u64 {
        let mut g = ready();
        for q in 0..4u8 {
            let mut h = [0u8; NUM_RESOURCES];
            for i in 0..NUM_RESOURCES {
                h[i] = rng.below(4) as u8;
            }
            set_hand(&mut g, q, h);
        }
        let _ = seed;
        for gi in 0..NUM_RESOURCES {
            for wi in 0..NUM_RESOURCES {
                if gi == wi || g.players[0].hand[gi] == 0 {
                    continue;
                }
                let mut give = [0u8; NUM_RESOURCES];
                give[gi] = 1;
                let mut want = [0u8; NUM_RESOURCES];
                want[wi] = 1;
                let a = Action::OfferTrade { give, want };
                let Some(d) = eval_delta(&g, 0, a, &w, &threats(&g, &w)) else { continue };
                let sim = preview(&g, 0, a, &w).unwrap();
                let full = evaluate(&sim, 0, &w) - evaluate(&g, 0, &w);
                assert!(
                    (d - full).abs() < 0.01,
                    "差分 {d} と全計算 {full} がズレている"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 50, "検査した組み合わせが少なすぎる: {checked}");
    let _ = (SHEEP, WHEAT, ORE);
    let _: NodeId = 0;
}
