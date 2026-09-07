//! M1/M3: 秘密情報の非干渉。
//!
//! 同じ観測履歴を生む複数の本番状態（相手の手札の分け方・発展カードの種類・山の内訳・
//! 本番 RNG・内部の並び）に対して、**観測・出来事・推定・判断・説明**が一致すること。
//! 公開情報そのものが変わる対は使わない（設計書 14.1）。

use catan_ai::agent_v2::{AgentV2, V2Config};
use catan_ai::belief::BeliefConfig;
use catan_ai::bots::{Bot, GreedyEvalBot, WeightedRandomBot};
use catan_core::action::{Action, DevCard, NUM_DEV_KINDS};
use catan_core::game::{Forced, Game, GameConfig};
use catan_core::observation::LEGACY_APP;
use catan_core::observer::{observe, project_event};
use catan_core::rng::Rng;
use catan_core::view::View;

/// 公開情報を保ったまま、秘密だけを差し替えた「双子」を作る。
///
/// - 相手（viewer 以外）どうしで資源の中身を入れ替える（枚数は不変）
/// - 相手の発展カードの種類を山と入れ替える（枚数・使用済みは不変。今手番購入分の枚数も不変）
/// - 本番 RNG を別物にする
fn twin(g: &Game, viewer: u8) -> Game {
    let mut t = g.clone();
    let n = g.n();
    let others: Vec<usize> = (0..n).filter(|&q| q as u8 != viewer).collect();
    // 資源: 相手全員の手札を合算して、枚数を保ったまま別の分け方にする（決定的に）
    let mut pool = [0u8; 5];
    for &q in &others {
        for r in 0..5 {
            pool[r] += t.players[q].hand[r];
        }
    }
    let mut bag: Vec<u8> = Vec::new();
    for r in 0..5 {
        for _ in 0..pool[r] {
            bag.push(r as u8);
        }
    }
    // 逆順に配る = 元と違う分け方（同じになることもあるが、その場合も比較は有効）
    bag.reverse();
    let mut k = 0;
    for &q in &others {
        let size = t.players[q].hand_size() as usize;
        let mut h = [0u8; 5];
        for _ in 0..size {
            h[bag[k] as usize] += 1;
            k += 1;
        }
        t.players[q].hand = h;
    }
    // 発展: 相手の札を山へ戻し、山から別の種類を配る（枚数保存）
    for &q in &others {
        let ps = &mut t.players[q];
        for i in 0..NUM_DEV_KINDS {
            t.dev_deck[i] += ps.dev[i];
        }
        let total = ps.dev_count();
        let bought: u8 = ps.dev_bought_this_turn.iter().sum();
        ps.dev = [0; NUM_DEV_KINDS];
        ps.dev_bought_this_turn = [0; NUM_DEV_KINDS];
        // 山の「多い種類から」順に配る（元は無作為なので大抵違う内訳になる）
        let mut left = total;
        let mut b_left = bought;
        let mut order: Vec<usize> = (0..NUM_DEV_KINDS).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(t.dev_deck[i]));
        for &i in &order {
            while left > 0 && t.dev_deck[i] > 0 {
                t.dev_deck[i] -= 1;
                t.players[q].dev[i] += 1;
                if b_left > 0 {
                    t.players[q].dev_bought_this_turn[i] += 1;
                    b_left -= 1;
                }
                left -= 1;
            }
        }
        assert_eq!(left, 0);
    }
    t.rng_dice = Rng::new(0xD1CE);
    t.rng_dev = Rng::new(0xDE7);
    t.rng_steal = Rng::new(0x57EA);
    t
}

/// 公開情報が同じであることの確認（双子の作り方の検査）
fn same_public(a: &Game, b: &Game) {
    assert_eq!(a.board, b.board);
    assert_eq!(a.bank, b.bank);
    for q in 0..a.n() {
        assert_eq!(a.players[q].hand_size(), b.players[q].hand_size());
        assert_eq!(a.players[q].dev_count(), b.players[q].dev_count());
        assert_eq!(a.players[q].played_dev, b.players[q].played_dev);
        assert_eq!(a.public_vp(q as u8), b.public_vp(q as u8));
    }
    assert_eq!(a.prompt, b.prompt);
    assert_eq!(a.to_act, b.to_act);
    assert_eq!(a.trade, b.trade);
}

#[test]
fn 秘密を差し替えても観測は一致する() {
    let mut checked = 0;
    for seed in 0..12u64 {
        let mut g = Game::with_config(4, seed, GameConfig::default());
        let mut bots: Vec<Box<dyn Bot>> = (0..4).map(|i| Box::new(GreedyEvalBot::new(seed + i)) as Box<dyn Bot>).collect();
        let mut buf = Vec::new();
        for step in 0..1500 {
            if g.is_over() {
                break;
            }
            g.legal_actions_into(&mut buf);
            if step % 37 == 0 && !g.is_setup() {
                let viewer = g.to_act;
                let t = twin(&g, viewer);
                same_public(&g, &t);
                let oa = observe(&g, viewer, &buf, LEGACY_APP, step as u64);
                let ob = observe(&t, viewer, &buf, LEGACY_APP, step as u64);
                assert_eq!(oa, ob, "seed={seed} step={step}: 秘密の差し替えで観測が変わった");
                // 相手の視点でも: 自分（viewer）の秘密は相手の観測に入らない
                for q in 0..4u8 {
                    if q == viewer {
                        continue;
                    }
                    // viewer の手札だけ変えた双子
                    let mut u = g.clone();
                    let me = &mut u.players[viewer as usize];
                    if me.hand_size() >= 2 && me.hand[0] > 0 {
                        // 木 1 枚を土に（銀行と交換して枚数と公開を保つ）
                        if u.bank[1] > 0 {
                            u.players[viewer as usize].hand[0] -= 1;
                            u.players[viewer as usize].hand[1] += 1;
                            u.bank[0] += 1;
                            u.bank[1] -= 1;
                            // 銀行が変わると公開情報が変わるので、この対は使わない
                            continue;
                        }
                    }
                    let oq = observe(&g, q, &buf, LEGACY_APP, step as u64);
                    let ot = observe(&t, q, &buf, LEGACY_APP, step as u64);
                    // t では q 自身の手札も入れ替わっているので own は違ってよい。public は一致
                    assert_eq!(oq.public, ot.public, "seed={seed} step={step} q={q}: 公開部分が変わった");
                }
                checked += 1;
            }
            let seat = g.to_act as usize;
            let a = bots[seat].decide(&View::new(&g, seat as u8), &buf);
            g.apply(a);
        }
    }
    assert!(checked > 30, "検査した局面が少ない: {checked}");
}

#[test]
fn 秘密を差し替えても投影された出来事は一致する() {
    for seed in 0..8u64 {
        let mut g = Game::with_config(4, seed, GameConfig::default());
        let mut bots: Vec<Box<dyn Bot>> = vec![
            Box::new(GreedyEvalBot::new(seed)),
            Box::new(WeightedRandomBot::new(seed + 1)),
            Box::new(GreedyEvalBot::new(seed + 2)),
            Box::new(WeightedRandomBot::new(seed + 3)),
        ];
        let mut buf = Vec::new();
        let mut seq = 0u64;
        for step in 0..1200 {
            if g.is_over() {
                break;
            }
            g.legal_actions_into(&mut buf);
            let seat = g.to_act as usize;
            let a = bots[seat].decide(&View::new(&g, seat as u8), &buf);
            // 出来事の偶然（出目・引いた札・盗んだ札）は元の対局で起きたものに固定して双子にも適用する。
            // 双子では「引いた札」「盗んだ札」が合法でないこともある（種類が違う）ので、
            // 第三者の視点で比較できる出来事だけを見る
            let before = g.clone();
            let rec = g.apply(a);
            seq += 1;
            let forced = match rec.result {
                catan_core::action::Outcome::Dice(x, y) => Forced::Dice(x, y),
                catan_core::action::Outcome::Drew(c) => Forced::Draw(c),
                catan_core::action::Outcome::Stole(s) => Forced::Steal(s),
                catan_core::action::Outcome::None => Forced::No,
            };
            if step % 11 != 0 || before.is_setup() {
                continue;
            }
            for viewer in 0..4u8 {
                if viewer == rec.actor {
                    continue; // 本人の視点は本人の秘密（引いた札）が違うので比較しない
                }
                let tb = twin(&before, viewer);
                // 双子で同じ手が指せるか（払える・持っている）。指せない対は公開情報が違うので使わない
                let ok = tb.can_apply(&a)
                    && match (a, forced) {
                        (Action::BuyDevCard, Forced::Draw(c)) => tb.dev_deck[c.idx()] > 0,
                        (Action::MoveRobber { victim: Some(v), .. }, Forced::Steal(Some(r))) => tb.players[v as usize].hand[r.idx()] > 0 && v != viewer,
                        (Action::MoveRobber { victim: Some(v), .. }, _) => v != viewer,
                        // 承諾者・対案者が「その札を持つ」ことは公開情報。素朴な双子はそれを壊すので、
                        // 払えない双子は「公開情報が違う対」として使わない
                        (Action::ConfirmTrade(partner), _) => {
                            let t = tb.trade.unwrap();
                            (0..5).all(|r| tb.players[partner as usize].hand[r] >= t.want[r])
                                && (0..5).all(|r| tb.players[t.proposer as usize].hand[r] >= t.give[r])
                        }
                        (Action::AcceptCounter { from, alt }, _) => {
                            let t = tb.trade.unwrap();
                            let (cg, cw) = t.counters[from as usize][alt as usize].unwrap();
                            (0..5).all(|r| tb.players[from as usize].hand[r] >= cg[r])
                                && (0..5).all(|r| tb.players[t.proposer as usize].hand[r] >= cw[r])
                        }
                        _ => true,
                    };
                if !ok {
                    continue;
                }
                let mut ta = tb.clone();
                let rec2 = ta.apply_forced(a, forced);
                let ea = project_event(&before, &g, &rec, viewer, LEGACY_APP, seq);
                let eb = project_event(&tb, &ta, &rec2, viewer, LEGACY_APP, seq);
                // 双子では相手の手札の中身が違うので、独占の「各人が渡した枚数」など公開の中身が変わる対は
                // 「公開情報が変わった」対として比較から外す。それ以外は完全一致
                if matches!(a, Action::PlayMonopoly(_)) {
                    continue;
                }
                if let (catan_core::observation::VisibleEvent::Roll { produced: pa, .. }, catan_core::observation::VisibleEvent::Roll { produced: pb, .. }) = (ea.payload, eb.payload) {
                    // 産出は盤と建物で決まるので双子でも同じ
                    assert_eq!(pa, pb);
                }
                assert_eq!(ea, eb, "seed={seed} step={step} viewer={viewer}: 秘密の差し替えで出来事が変わった {a:?}");
            }
        }
    }
}

/// 同じ出来事の列を流した 2 つのエージェントは、秘密だけ違う局面で同じ判断・同じ説明をする。
/// 本番 RNG を変えた双子で、続く手の偶然を固定して比較する。
#[test]
fn 秘密を差し替えても判断と説明は一致する() {
    let mut compared = 0;
    for seed in 0..10u64 {
        let mut g = Game::with_config(4, seed, GameConfig::default());
        let mut bots: Vec<Box<dyn Bot>> = vec![
            Box::new(GreedyEvalBot::new(seed)),
            Box::new(WeightedRandomBot::new(seed + 1)),
            Box::new(GreedyEvalBot::new(seed + 2)),
            Box::new(WeightedRandomBot::new(seed + 3)),
        ];
        let viewer = (seed % 4) as u8;
        let cfg = V2Config { belief: BeliefConfig { particles: 64, ..Default::default() }, ..V2Config::default() };
        let mut agent_a = AgentV2::new(1234, cfg);
        let mut agent_b = AgentV2::new(1234, cfg);
        let mut buf = Vec::new();
        let mut seq = 0u64;
        for step in 0..1500 {
            if g.is_over() {
                break;
            }
            g.legal_actions_into(&mut buf);
            if g.to_act == viewer && !g.is_setup() && step % 3 == 0 {
                let t = twin(&g, viewer);
                same_public(&g, &t);
                let oa = observe(&g, viewer, &buf, LEGACY_APP, seq);
                let ob = observe(&t, viewer, &buf, LEGACY_APP, seq);
                assert_eq!(oa, ob);
                let da = agent_a.decide(&oa);
                let db = agent_b.decide(&ob);
                assert_eq!(da, db, "seed={seed} step={step}: 判断が変わった");
                assert_eq!(agent_a.explain(), agent_b.explain(), "seed={seed} step={step}: 説明が変わった");
                let ba = agent_a.belief.as_ref().unwrap();
                let bb = agent_b.belief.as_ref().unwrap();
                for q in 0..4u8 {
                    assert_eq!(ba.vp_dist(q), bb.vp_dist(q), "seed={seed} step={step}: 予測が変わった");
                }
                compared += 1;
            }
            let seat = g.to_act as usize;
            let a = bots[seat].decide(&View::new(&g, seat as u8), &buf);
            let before = g.clone();
            let rec = g.apply(a);
            seq += 1;
            let ev = project_event(&before, &g, &rec, viewer, LEGACY_APP, seq);
            agent_a.observe(&ev);
            agent_b.observe(&ev);
        }
    }
    assert!(compared > 40, "比較した判断が少ない: {compared}");
}

/// 相手の伏せ札を勝利点↔騎士で入れ替えても、v2 の判断は変わらない（旧テストと同じ型）
#[test]
fn 相手の伏せた勝利点はv2の判断を変えない() {
    for seed in 0..20u64 {
        let decide = |kind: DevCard| {
            let mut g = Game::with_config(4, seed, GameConfig::default());
            let mut rng = Rng::with_stream(seed, 77);
            let mut buf = Vec::new();
            while g.is_setup() {
                g.legal_actions_into(&mut buf);
                g.apply(buf[rng.below(buf.len() as u32) as usize]);
            }
            g.players[1].dev = [0; 5];
            g.players[1].dev[kind.idx()] = 2;
            g.dev_deck[kind.idx()] -= 2;
            g.legal_actions_into(&mut buf);
            let seat = g.to_act;
            let obs = observe(&g, seat, &buf, LEGACY_APP, 0);
            let mut agent = AgentV2::new(seed * 31 + 7, V2Config::default());
            agent.decide(&obs)
        };
        assert_eq!(decide(DevCard::VictoryPoint), decide(DevCard::Knight), "seed {seed}");
    }
}
