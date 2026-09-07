//! M2: 推定（Belief）の必須テスト（設計書 6.6 と保存則）。
//!
//! ここでは本番 `Game` を**採点器**として使う。エージェント側には渡さない。

use catan_ai::belief::{Belief, BeliefConfig};
use catan_ai::bots::{Bot, GreedyEvalBot, RandomBot, WeightedRandomBot};
use catan_core::action::{Action, ActionRecord, DevCard, Outcome, Prompt, DEV_DECK_COMPOSITION, NUM_DEV_KINDS};
use catan_core::board::{PlayerId, NUM_RESOURCES};
use catan_core::game::{Forced, Game, GameConfig, BANK_PER_RESOURCE};
use catan_core::observation::{EventAftermath, EventContext, ObservedEvent, VisibleEvent, LEGACY_APP};
use catan_core::observer::project_event;
use catan_core::rng::Rng;
use catan_core::view::View;

/// 行動を 1 つ適用し、全席の Belief に出来事を配る
fn step(g: &mut Game, a: Action, forced: Forced, beliefs: &mut [Belief], seq: &mut u64) -> ActionRecord {
    let before = g.clone();
    let rec = g.apply_forced(a, forced);
    *seq += 1;
    for b in beliefs.iter_mut() {
        let ev = project_event(&before, g, &rec, b.viewer, LEGACY_APP, *seq);
        b.observe(&ev);
    }
    rec
}

fn check_conservation(b: &Belief, g: &Game, ctx: &str) {
    let me = b.viewer as usize;
    let mut alive = 0;
    for pt in &b.res {
        if pt.w <= 0.0 {
            continue;
        }
        alive += 1;
        // 資源 19 枚
        for r in 0..NUM_RESOURCES {
            let held: u32 = (0..g.n()).map(|q| pt.hand[q][r] as u32).sum();
            assert_eq!(held + g.bank[r] as u32, BANK_PER_RESOURCE as u32, "{ctx}: 資源 {r} の保存則");
        }
        for q in 0..g.n() {
            assert_eq!(pt.hand[q].iter().sum::<u8>(), g.players[q].hand_size(), "{ctx}: P{q} の手札枚数");
        }
        assert_eq!(pt.hand[me], g.players[me].hand, "{ctx}: 自分の手札");
    }
    assert!(alive > 0, "{ctx}: 生きている資源粒子が無い");
    let mut alive = 0;
    for pt in &b.dev {
        if pt.w <= 0.0 {
            continue;
        }
        alive += 1;
        for q in 0..g.n() {
            assert_eq!(pt.dev_total(q as PlayerId), g.players[q].dev_count(), "{ctx}: P{q} の発展枚数");
        }
        assert_eq!(pt.dev(me as PlayerId), g.players[me].dev, "{ctx}: 自分の発展");
        // 発展 25 枚（種類別）。使用済み進歩カードはゲームから除外される
        for (c, n) in DEV_DECK_COMPOSITION {
            let held: u32 = (0..g.n()).map(|q| pt.dev(q as PlayerId)[c.idx()] as u32).sum();
            let played: u32 = (0..g.n()).map(|q| g.players[q].played_dev[c.idx()] as u32).sum();
            assert_eq!(held + played + pt.deck[c.idx()] as u32, n as u32, "{ctx}: {} の保存則", c.ja());
        }
        let vps: u8 = (0..g.n()).map(|q| pt.hidden_vp(q as PlayerId)).sum::<u8>() + pt.deck[DevCard::VictoryPoint.idx()];
        assert!(vps <= 5, "{ctx}: 勝利点が 5 を超えた");
        assert_eq!(pt.deck.iter().sum::<u8>(), g.dev_deck.iter().sum::<u8>(), "{ctx}: 山の枚数");
    }
    assert!(alive > 0, "{ctx}: 生きている発展粒子が無い");
    assert_eq!(b.diag.inconsistent, 0, "{ctx}: 観測と矛盾した粒子があった");
}

#[test]
fn 保存則と観測整合を全粒子が満たす() {
    for seed in 0..6u64 {
        let mut g = Game::with_config(4, seed, GameConfig::default());
        let mut bots: Vec<Box<dyn Bot>> = vec![
            Box::new(GreedyEvalBot::new(seed)),
            Box::new(WeightedRandomBot::new(seed + 1)),
            Box::new(GreedyEvalBot::new(seed + 2)),
            Box::new(RandomBot::new(seed + 3)),
        ];
        let mut beliefs: Vec<Belief> = (0..4).map(|v| Belief::new(v, 4, BeliefConfig { particles: 64, ..Default::default() }, seed * 10 + v as u64)).collect();
        let mut seq = 0u64;
        let mut buf = Vec::new();
        let mut steps = 0;
        while !g.is_over() && steps < 3000 && g.turn < 120 {
            g.legal_actions_into(&mut buf);
            let seat = g.to_act as usize;
            let a = bots[seat].decide(&View::new(&g, seat as u8), &buf);
            step(&mut g, a, Forced::No, &mut beliefs, &mut seq);
            steps += 1;
            if steps % 7 == 0 {
                for b in &beliefs {
                    check_conservation(b, &g, &format!("seed={seed} step={steps} viewer={}", b.viewer));
                }
            }
        }
        for b in &beliefs {
            check_conservation(b, &g, &format!("seed={seed} 終局 viewer={}", b.viewer));
            assert!(b.diag.min_ess_res > 0.0 && b.diag.min_ess_dev > 0.0);
        }
    }
}

/// 初期配置を済ませて P0 の手番（サイコロ前）にする
fn ready(seed: u64, beliefs: &mut [Belief], seq: &mut u64) -> Game {
    let mut g = Game::with_config(4, seed, GameConfig::default());
    let mut rng = Rng::with_stream(seed, 5);
    let mut buf = Vec::new();
    while g.is_setup() {
        g.legal_actions_into(&mut buf);
        let a = buf[rng.below(buf.len() as u32) as usize];
        step(&mut g, a, Forced::No, beliefs, seq);
    }
    g
}

/// 手番プレイヤーに資源を持たせる（銀行から。公開情報として全 Belief に「産出」で届ける）
fn grant(g: &mut Game, p: PlayerId, bundle: [u8; 5], beliefs: &mut [Belief], seq: &mut u64) {
    // サイコロ以外で資源を渡す手段が無いので、Roll を強制の出目で適用した上で差し替える。
    // 代わりに「収穫」として観測させるため、before/after を作って直接イベントを配る
    let before = g.clone();
    for r in 0..5 {
        g.bank[r] -= bundle[r];
        g.players[p as usize].hand[r] += bundle[r];
    }
    *seq += 1;
    let rec = ActionRecord { actor: p, action: Action::Roll, result: Outcome::Dice(1, 1) };
    for b in beliefs.iter_mut() {
        let ev = project_event(&before, g, &rec, b.viewer, LEGACY_APP, *seq);
        b.observe(&ev);
    }
}

fn hypergeom(n_total: u32, k_vp: u32, m: u32) -> [f32; 6] {
    let comb = |n: u32, k: u32| -> f64 {
        if k > n {
            return 0.0;
        }
        let k = k.min(n - k);
        let mut r = 1.0f64;
        for i in 0..k {
            r = r * (n - i) as f64 / (i + 1) as f64;
        }
        r
    };
    let mut out = [0f32; 6];
    for k in 0..=5u32 {
        if k <= m && k <= k_vp && m - k <= n_total - k_vp {
            out[k as usize] = (comb(k_vp, k) * comb(n_total - k_vp, m - k) / comb(n_total, m)) as f32;
        }
    }
    out
}

/// 「行動情報なし」の小例で、粒子の分布が超幾何分布と一致する
#[test]
fn 追加情報がなければ超幾何分布と一致する() {
    let mut beliefs: Vec<Belief> = (0..4).map(|v| Belief::new(v, 4, BeliefConfig { particles: 4096, ..BeliefConfig::no_history() }, 77 + v as u64)).collect();
    let mut seq = 0;
    let mut g = ready(11, &mut beliefs, &mut seq);
    // P0 の手番。ダイスは 2 を強制（産出はほぼ無い）
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
    // P0 が発展カードを 3 枚買う（山からの実際の種類は観測されない）
    grant(&mut g, 0, [0, 0, 3, 3, 3], &mut beliefs, &mut seq);
    for _ in 0..3 {
        assert!(g.legal_actions().contains(&Action::BuyDevCard));
        step(&mut g, Action::BuyDevCard, Forced::No, &mut beliefs, &mut seq);
    }
    // P1 から見ると: 未知 25 枚のうち VP は 5 枚、P0 が 3 枚持つ
    let expect = hypergeom(25, 5, 3);
    let got = beliefs[1].vp_dist(0);
    for k in 0..6 {
        assert!((got[k] - expect[k]).abs() < 0.03, "k={k}: 粒子 {:.3} 超幾何 {:.3}", got[k], expect[k]);
    }
    // 履歴を使う版でも、買った手番の不使用は証拠にならない（EndTurn までは同じ）
    let mut full: Vec<Belief> = (0..4).map(|v| Belief::new(v, 4, BeliefConfig { particles: 4096, ..BeliefConfig::full() }, 77 + v as u64)).collect();
    let mut seq2 = 0;
    let mut g2 = ready(11, &mut full, &mut seq2);
    step(&mut g2, Action::Roll, Forced::Dice(1, 1), &mut full, &mut seq2);
    grant(&mut g2, 0, [0, 0, 3, 3, 3], &mut full, &mut seq2);
    for _ in 0..3 {
        step(&mut g2, Action::BuyDevCard, Forced::No, &mut full, &mut seq2);
    }
    step(&mut g2, Action::EndTurn, Forced::No, &mut full, &mut seq2);
    let got2 = full[1].vp_dist(0);
    for k in 0..6 {
        assert!((got2[k] - expect[k]).abs() < 0.03, "買った当日の不使用で分布が動いた k={k}: {:.3} vs {:.3}", got2[k], expect[k]);
    }
}

/// P0 が 1 枚買って、使う機会のある自分の手番を何度も何もせず終えると VP 確率が上がる。
/// 買った当日・別カード使用済みの手番では上がらない。
#[test]
fn 有効な機会の不使用でだけvp確率が上がる() {
    let cfg = BeliefConfig { particles: 2048, ..BeliefConfig::features_only() };
    let mut beliefs: Vec<Belief> = (0..4).map(|v| Belief::new(v, 4, cfg, 5 + v as u64)).collect();
    let mut seq = 0;
    let mut g = ready(21, &mut beliefs, &mut seq);
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
    grant(&mut g, 0, [0, 0, 1, 1, 1], &mut beliefs, &mut seq);
    step(&mut g, Action::BuyDevCard, Forced::Draw(DevCard::VictoryPoint), &mut beliefs, &mut seq);
    let prior = beliefs[1].vp_dist(0)[1];
    assert!((prior - 0.2).abs() < 0.03, "事前は 5/25: {prior}");
    // 買った当日の EndTurn: 動かない
    step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
    let after_same_day = beliefs[1].vp_dist(0)[1];
    assert!((after_same_day - prior).abs() < 0.01, "買った当日の不使用で動いた: {prior} → {after_same_day}");
    // 他の 3 人の手番を消化（何もしない）
    let mut last = after_same_day;
    for round in 0..3 {
        for _ in 1..4 {
            step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
            step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
        }
        // P0 の手番: 何もしないで終える（有効な機会）
        step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
        step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
        let now = beliefs[1].vp_dist(0)[1];
        assert!(now > last + 0.02, "巡 {round}: 不使用で VP 確率が上がらない {last} → {now}");
        last = now;
    }
    assert!(last > 0.35, "3 機会の不使用の後の VP 確率が低すぎる: {last}");
    assert!(last < 0.95, "3 機会で確定扱いになっている: {last}");
}

#[test]
fn 別カードを使った手番の不使用は証拠にならない() {
    let cfg = BeliefConfig { particles: 2048, ..BeliefConfig::features_only() };
    let mut beliefs: Vec<Belief> = (0..4).map(|v| Belief::new(v, 4, cfg, 9 + v as u64)).collect();
    let mut seq = 0;
    let mut g = ready(23, &mut beliefs, &mut seq);
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
    grant(&mut g, 0, [0, 0, 2, 2, 2], &mut beliefs, &mut seq);
    step(&mut g, Action::BuyDevCard, Forced::Draw(DevCard::Knight), &mut beliefs, &mut seq);
    step(&mut g, Action::BuyDevCard, Forced::Draw(DevCard::VictoryPoint), &mut beliefs, &mut seq);
    step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
    for _ in 1..4 {
        step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
        step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
    }
    // P0 の手番: 騎士を使う → もう 1 枚については証拠にならない
    let before = beliefs[1].vp_dist(0);
    assert!(g.legal_actions().contains(&Action::PlayKnight));
    step(&mut g, Action::PlayKnight, Forced::No, &mut beliefs, &mut seq);
    // 盗賊の移動（誰からも奪えない場所か、最初の候補）
    let mv = g.legal_actions()[0];
    step(&mut g, mv, Forced::Steal(None), &mut beliefs, &mut seq);
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
    step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
    let after = beliefs[1].vp_dist(0);
    // 騎士が 1 枚消えたので「残り 1 枚が VP」の確率は条件付けで動くが、
    // 「使わなかった」尤度は掛かっていない: 残り 1 枚の VP 確率は超幾何の条件付き (5/24 付近) を大きく超えない
    let p_vp_after = after[1];
    assert!(p_vp_after < 0.35, "別カード使用済みの手番の不使用が証拠に使われた: {p_vp_after} (前 {:?})", before);
}

#[test]
fn 置ける道がない時の街道建設の不使用は証拠にならない() {
    // 道の駒を使い切った P0。街道建設の q は 0 なので、VP と街道の比は事前のまま
    let cfg = BeliefConfig { particles: 4096, ..BeliefConfig::features_only() };
    let mut beliefs: Vec<Belief> = (0..4).map(|v| Belief::new(v, 4, cfg, 13 + v as u64)).collect();
    let mut seq = 0;
    let mut g = ready(31, &mut beliefs, &mut seq);
    g.players[0].roads_left = 0;
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
    grant(&mut g, 0, [0, 0, 1, 1, 1], &mut beliefs, &mut seq);
    step(&mut g, Action::BuyDevCard, Forced::Draw(DevCard::RoadBuilding), &mut beliefs, &mut seq);
    let dist = |b: &Belief| -> [f32; NUM_DEV_KINDS] {
        let mut out = [0f32; NUM_DEV_KINDS];
        for pt in &b.dev {
            let d = pt.dev(0);
            for i in 0..NUM_DEV_KINDS {
                out[i] += pt.w * d[i] as f32;
            }
        }
        out
    };
    let before = dist(&beliefs[1]);
    step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
    for _ in 0..2 {
        for _ in 1..4 {
            step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
            step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
        }
        step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
        step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
    }
    let after = dist(&beliefs[1]);
    let rb = DevCard::RoadBuilding.idx();
    let vp = DevCard::VictoryPoint.idx();
    let ratio_before = before[rb] / before[vp];
    let ratio_after = after[rb] / after[vp];
    assert!((ratio_before - ratio_after).abs() < 0.12, "街道:VP の比が動いた {ratio_before:.3} → {ratio_after:.3}");
    // 騎士の確率は下がっている（使えたのに使わなかった）
    assert!(after[DevCard::Knight.idx()] < before[DevCard::Knight.idx()] - 0.05);
}

#[test]
fn 傾向の混合は過信を抑える() {
    let run = |cfg: BeliefConfig| -> f32 {
        let mut beliefs: Vec<Belief> = (0..4).map(|v| Belief::new(v, 4, cfg, 17 + v as u64)).collect();
        let mut seq = 0;
        let mut g = ready(41, &mut beliefs, &mut seq);
        step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
        grant(&mut g, 0, [0, 0, 1, 1, 1], &mut beliefs, &mut seq);
        step(&mut g, Action::BuyDevCard, Forced::Draw(DevCard::Knight), &mut beliefs, &mut seq);
        step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
        for _ in 0..6 {
            for _ in 1..4 {
                step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
                step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
            }
            step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
            step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
        }
        beliefs[1].vp_dist(0)[1]
    };
    let with_styles = run(BeliefConfig { particles: 4096, ..BeliefConfig::full() });
    let without = run(BeliefConfig { particles: 4096, ..BeliefConfig::features_only() });
    assert!(with_styles < without, "傾向の混合が過信を抑えていない: 混合 {with_styles:.3} ≥ 単一 {without:.3}");
    assert!(with_styles < 0.97, "6 機会で確定扱い: {with_styles}");
}

#[test]
fn 複数購入後の使用で世代を決め打ちしない() {
    let cfg = BeliefConfig { particles: 512, ..BeliefConfig::no_history() };
    let mut beliefs: Vec<Belief> = (0..4).map(|v| Belief::new(v, 4, cfg, 19 + v as u64)).collect();
    let mut seq = 0;
    let mut g = ready(43, &mut beliefs, &mut seq);
    // 手番 A: 1 枚
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
    grant(&mut g, 0, [0, 0, 1, 1, 1], &mut beliefs, &mut seq);
    step(&mut g, Action::BuyDevCard, Forced::Draw(DevCard::Knight), &mut beliefs, &mut seq);
    let turn_a = g.turn;
    step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
    for _ in 1..4 {
        step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
        step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
    }
    // 手番 B: もう 1 枚
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
    grant(&mut g, 0, [0, 0, 1, 1, 1], &mut beliefs, &mut seq);
    step(&mut g, Action::BuyDevCard, Forced::Draw(DevCard::Knight), &mut beliefs, &mut seq);
    let turn_b = g.turn;
    step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
    for _ in 1..4 {
        step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
        step(&mut g, Action::EndTurn, Forced::No, &mut beliefs, &mut seq);
    }
    // 手番 C: 騎士を使う
    step(&mut g, Action::PlayKnight, Forced::No, &mut beliefs, &mut seq);
    let mv = g.legal_actions()[0];
    step(&mut g, mv, Forced::Steal(None), &mut beliefs, &mut seq);
    let b = &beliefs[1];
    let (mut from_a, mut from_b) = (0.0f32, 0.0f32);
    for pt in b.dev.iter().filter(|pt| pt.w > 0.0) {
        let cs = &pt.cohorts[0];
        let a_left = cs.iter().filter(|c| c.bought_turn == turn_a).map(|c| c.total()).sum::<u8>();
        let b_left = cs.iter().filter(|c| c.bought_turn == turn_b).map(|c| c.total()).sum::<u8>();
        assert_eq!(a_left + b_left, 1, "残りは 1 枚のはず");
        if a_left == 1 {
            from_b += pt.w; // B の札が使われた仮説
        } else {
            from_a += pt.w;
        }
    }
    assert!(from_a > 0.2 && from_b > 0.2, "どちらの世代から使われたかを決め打ちしている: A={from_a:.2} B={from_b:.2}");
}

/// 勝利判定は手番プレイヤーに対してだけ行われる。他人の番で 10 点相当でも除外しない
#[test]
fn 勝利判定の証拠は正しいタイミングだけ() {
    let cfg = BeliefConfig { particles: 1024, ..BeliefConfig::no_history() };
    // 同じ履歴を 2 つの Belief（どちらも P0 席）に流し、最後の 1 出来事だけ変える
    let mut bs = vec![Belief::new(0, 4, cfg, 23), Belief::new(0, 4, cfg, 23)];
    let mut seq = 0;
    let mut g = ready(47, &mut bs, &mut seq);
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut bs, &mut seq);
    grant(&mut g, 1, [0, 0, 2, 2, 2], &mut bs, &mut seq);
    step(&mut g, Action::EndTurn, Forced::No, &mut bs, &mut seq);
    // P1 の手番で 2 枚買う
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut bs, &mut seq);
    step(&mut g, Action::BuyDevCard, Forced::Draw(DevCard::Knight), &mut bs, &mut seq);
    step(&mut g, Action::BuyDevCard, Forced::Draw(DevCard::Knight), &mut bs, &mut seq);
    let before = bs[0].vp_dist(1);
    assert!(before[0] < 0.9, "事前: {before:?}");
    // 合成した出来事: P1 の公開点が 9 で、手番プレイヤーが `turn_player`
    let mk = |turn_player: PlayerId, g: &Game| -> ObservedEvent {
        let mut after = EventAftermath {
            turn_player,
            to_act: turn_player,
            prompt: Prompt::PlayTurn,
            winner: None,
            dev_deck_left: g.dev_deck.iter().sum(),
            bank: Some(g.bank),
            ..EventAftermath::default()
        };
        for q in 0..4 {
            after.hand_size[q] = g.players[q].hand_size();
            after.dev_count[q] = g.players[q].dev_count();
            after.public_vp[q] = g.public_vp(q as u8);
        }
        after.public_vp[1] = 9;
        ObservedEvent {
            seq: seq + 1,
            viewer: 0,
            actor: 2,
            prompt: Prompt::PlayTurn,
            turn: g.turn,
            turn_player,
            num_players: 4,
            payload: VisibleEvent::Hidden,
            before: EventContext::default(),
            after,
        }
    };
    // 分岐 1: P2 の手番中に P1 が公開 9 点 → 除外しない
    bs[0].observe(&mk(2, &g));
    let d = bs[0].vp_dist(1);
    assert!(d[0] < 0.9, "他人の番で 10 点相当になっただけで除外した: {d:?}");
    // 分岐 2: 手番が P1 に回ったのに勝っていない → P1 の伏せ VP は 0
    bs[1].observe(&mk(1, &g));
    let d = bs[1].vp_dist(1);
    assert!((d[0] - 1.0).abs() < 1e-4, "手番プレイヤーが勝っていない証拠が使われていない: {d:?}");
}

/// 産出はすべて公開なので、盗みが無ければ相手の手札は確定する。
/// 自分が当事者の盗みは種類を知り、第三者の盗みは種類を参照しない（枚数比で割れる）。
#[test]
fn 当事者と第三者で盗みの扱いが違う() {
    let cfg = BeliefConfig { particles: 512, ..BeliefConfig::no_history() };
    let mut beliefs: Vec<Belief> = (0..4).map(|v| Belief::new(v, 4, cfg, 29 + v as u64)).collect();
    let mut seq = 0;
    let mut g = ready(53, &mut beliefs, &mut seq);
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut beliefs, &mut seq);
    // ここまで全部公開（初期配置の産出）なので、P0 から見た P1 の手札は確定
    let h1 = g.players[1].hand;
    assert!((beliefs[0].prob_hand(1, &h1) - 1.0).abs() < 1e-5, "公開情報だけなら確定するはず");
    // 木を 2 枚足して、P1 が木 ≥ 2 かつ他の種類も持つようにする
    grant(&mut g, 1, [2, 0, 1, 0, 0], &mut beliefs, &mut seq);
    let h1 = g.players[1].hand;
    let total1 = h1.iter().sum::<u8>() as f32;
    // P0 が盗賊を P1 の建物のあるヘクスへ動かして木を奪う（強制）
    let mut tiles = Vec::new();
    for t in 0..19u8 {
        if t == g.board.robber {
            continue;
        }
        let topo = catan_core::topology::Topology::get();
        if topo.tile_nodes[t as usize].iter().any(|&nd| matches!(g.board.building[nd as usize], Some(b) if b.owner == 1)) {
            tiles.push(t);
        }
    }
    let t = tiles[0];
    g.prompt = Prompt::MoveRobber;
    g.to_act = 0;
    step(&mut g, Action::MoveRobber { tile: t, victim: Some(1) }, Forced::Steal(Some(catan_core::board::Resource::Wood)), &mut beliefs, &mut seq);
    let mut after = h1;
    after[0] -= 1;
    // 当事者 P0 と P1 は確定
    assert!((beliefs[0].prob_hand(1, &after) - 1.0).abs() < 1e-5, "奪った側は種類を知る");
    assert!((beliefs[1].prob_hand(0, &g.players[0].hand) - 1.0).abs() < 1e-5, "奪われた側も種類を知る");
    // 第三者 P3 は「P1 の手札の枚数比」で割れる
    let p_wood = beliefs[3].prob_hand(1, &after);
    let expect = h1[0] as f32 / total1;
    assert!((p_wood - expect).abs() < 0.08, "第三者の木の確率 {p_wood} 期待 {expect}");
    assert!(p_wood < 0.999, "第三者が種類を知っている");
}
