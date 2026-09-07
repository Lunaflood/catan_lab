//! v2 の計器: 推定の校正（Brier / log loss / 信頼度図）、相手モデルの当てはめ、判断の説明。
//!
//! ここは**採点器**。本番 `Game` の秘密を読むのはこのファイルの中だけで、
//! 推定（Belief）には出来事の投影しか渡さない。

use catan_ai::agent_v2::{AgentV2Bot, V2Config};
use catan_ai::belief::reach::ReachSolver;
use catan_ai::belief::{Belief, BeliefConfig};
use catan_ai::bots::Bot;
use catan_ai::opponent::{q_use, Opportunity, OpponentParams, STYLE_BALANCED};
use catan_core::action::{Action, DevCard, DEV_CARDS, NUM_DEV_KINDS};
use catan_core::board::{PlayerId, NUM_RESOURCES};
use catan_core::game::{Game, GameConfig};
use catan_core::observation::{VisibleEvent, LEGACY_APP};
use catan_core::observer::project_event;
use catan_core::view::View;

fn permutations(n: usize) -> Vec<Vec<usize>> {
    let mut cur: Vec<usize> = (0..n).collect();
    let mut out = vec![cur.clone()];
    loop {
        let Some(i) = (0..n.saturating_sub(1)).rev().find(|&i| cur[i] < cur[i + 1]) else { break };
        let j = (i + 1..n).rev().find(|&j| cur[j] > cur[i]).unwrap();
        cur.swap(i, j);
        cur[i + 1..].reverse();
        out.push(cur.clone());
    }
    out
}

/// 予測 1 件（隠れ勝利点）
struct VpPred {
    dist: [f32; 6],
    truth: u8,
    /// 相手が発展カードを持っていた枚数
    dev_count: u8,
}

/// 予測 1 件（次の手番で勝つ）
struct HazardPred {
    p: f32,
    truth: bool,
}

#[derive(Default)]
struct Stats {
    vp: Vec<VpPred>,
    hand_hit: Vec<f32>,
    hand_mae: Vec<f32>,
    hazard: Vec<HazardPred>,
    ess: Vec<f32>,
    collapses: u64,
    inconsistent: u64,
    resyncs: u64,
    events: u64,
    update_ns: u128,
    hazard_ns: u128,
    hazard_calls: u64,
}

fn brier(d: &[f32; 6], truth: u8) -> f32 {
    let mut s = 0.0;
    for k in 0..6 {
        let y = if k == truth as usize { 1.0 } else { 0.0 };
        s += (d[k] - y) * (d[k] - y);
    }
    s
}

fn report_calibration(label: &str, st: &Stats) -> String {
    let mut out = String::new();
    let n = st.vp.len().max(1) as f32;
    let b: f32 = st.vp.iter().map(|p| brier(&p.dist, p.truth)).sum::<f32>() / n;
    let ll: f32 = st.vp.iter().map(|p| -(p.dist[p.truth as usize].max(1e-4)).ln()).sum::<f32>() / n;
    // 基準: 超幾何（履歴なし）は別走で出すので、ここでは「一様 1/6」と「常に 0」を参考に
    let uniform_b: f32 = st.vp.iter().map(|p| brier(&[1.0 / 6.0; 6], p.truth)).sum::<f32>() / n;
    let zero_b: f32 = st.vp.iter().map(|p| brier(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0], p.truth)).sum::<f32>() / n;
    out.push_str(&format!("## {label}\n"));
    out.push_str(&format!("隠れ勝利点の予測 {} 件（相手が発展カードを 1 枚以上持つ手番終了時）\n", st.vp.len()));
    out.push_str(&format!("  Brier {b:.4}（一様 {uniform_b:.4} / 常に0 {zero_b:.4}）  log loss {ll:.4}\n"));
    // 信頼度図: P(隠れVP ≥ 1)
    let mut bins = [(0usize, 0.0f32, 0.0f32); 10];
    for p in &st.vp {
        let pr = 1.0 - p.dist[0];
        let i = ((pr * 10.0) as usize).min(9);
        bins[i].0 += 1;
        bins[i].1 += pr;
        bins[i].2 += if p.truth >= 1 { 1.0 } else { 0.0 };
    }
    out.push_str("  信頼度図 P(隠れVP≥1): 予測区間 / 件数 / 平均予測 / 実際の割合\n");
    for (i, (c, sp, st_)) in bins.iter().enumerate() {
        if *c > 0 {
            out.push_str(&format!(
                "    {:.1}-{:.1}  {:>6}  {:.3}  {:.3}\n",
                i as f32 / 10.0,
                (i + 1) as f32 / 10.0,
                c,
                sp / *c as f32,
                st_ / *c as f32
            ));
        }
    }
    // 発展カード枚数別
    for dc in 1..=4u8 {
        let sub: Vec<&VpPred> = st.vp.iter().filter(|p| if dc == 4 { p.dev_count >= 4 } else { p.dev_count == dc }).collect();
        if sub.is_empty() {
            continue;
        }
        let m = sub.len() as f32;
        let bb: f32 = sub.iter().map(|p| brier(&p.dist, p.truth)).sum::<f32>() / m;
        let ev: f32 = sub.iter().map(|p| p.dist.iter().enumerate().map(|(k, w)| k as f32 * w).sum::<f32>()).sum::<f32>() / m;
        let tv: f32 = sub.iter().map(|p| p.truth as f32).sum::<f32>() / m;
        out.push_str(&format!(
            "  発展 {}{} 枚: {} 件  Brier {bb:.4}  期待VP {ev:.3} / 実際 {tv:.3}\n",
            dc,
            if dc == 4 { "+" } else { "" },
            sub.len()
        ));
    }
    if !st.hand_hit.is_empty() {
        let m = st.hand_hit.len() as f32;
        out.push_str(&format!(
            "相手の資源手札: 正解の手札に置いた確率の平均 {:.3} / 期待枚数の平均絶対誤差 {:.3} 枚（{} 件）\n",
            st.hand_hit.iter().sum::<f32>() / m,
            st.hand_mae.iter().sum::<f32>() / m,
            st.hand_hit.len()
        ));
    }
    if !st.hazard.is_empty() {
        let m = st.hazard.len() as f32;
        let hb: f32 = st.hazard.iter().map(|h| (h.p - if h.truth { 1.0 } else { 0.0 }).powi(2)).sum::<f32>() / m;
        let base: f32 = st.hazard.iter().filter(|h| h.truth).count() as f32 / m;
        let base_b = base * (1.0 - base);
        out.push_str(&format!(
            "勝利ハザード（次の自分の手番で勝つ）: {} 件  Brier {hb:.4}（基準率 {base:.4} を常に出すと {base_b:.4}）\n",
            st.hazard.len()
        ));
        let mut bins = [(0usize, 0.0f32, 0.0f32); 10];
        for h in &st.hazard {
            let i = ((h.p * 10.0) as usize).min(9);
            bins[i].0 += 1;
            bins[i].1 += h.p;
            bins[i].2 += if h.truth { 1.0 } else { 0.0 };
        }
        out.push_str("  信頼度図: 予測区間 / 件数 / 平均予測 / 実際に勝った割合\n");
        for (i, (c, sp, st_)) in bins.iter().enumerate() {
            if *c > 0 {
                out.push_str(&format!(
                    "    {:.1}-{:.1}  {:>6}  {:.3}  {:.3}\n",
                    i as f32 / 10.0,
                    (i + 1) as f32 / 10.0,
                    c,
                    sp / *c as f32,
                    st_ / *c as f32
                ));
            }
        }
    }
    let ess_mean = if st.ess.is_empty() { 0.0 } else { st.ess.iter().sum::<f32>() / st.ess.len() as f32 };
    let ess_min = st.ess.iter().cloned().fold(f32::INFINITY, f32::min);
    out.push_str(&format!(
        "診断: 出来事 {} / 平均ESS {ess_mean:.1} / 最小ESS {ess_min:.1} / 全滅→組み直し {} / 観測不整合 {} / 再同期 {}\n",
        st.events, st.collapses, st.inconsistent, st.resyncs
    ));
    out.push_str(&format!(
        "時間: 1 出来事の更新 {:.1} µs / ハザード 1 回 {:.1} µs（{} 回）\n",
        st.update_ns as f64 / 1000.0 / st.events.max(1) as f64,
        st.hazard_ns as f64 / 1000.0 / st.hazard_calls.max(1) as f64,
        st.hazard_calls
    ));
    out
}

/// 校正: 対照 CPU の対局に受動的な Belief を付け、予測と正解（採点器だけが見る）を比べる
pub fn calib(args: &[String], make_bot: &dyn Fn(&str, u64) -> Option<Box<dyn Bot>>) {
    let mut games = 200usize;
    let mut seed = 5_000_000u64;
    let mut names: Vec<String> = vec!["v2_control".into(); 4];
    let mut particles = 256usize;
    let mut cfgs: Vec<(String, BeliefConfig)> = vec![
        ("履歴なし(超幾何)".into(), BeliefConfig::no_history()),
        ("保持期間だけ".into(), BeliefConfig::duration_only()),
        ("使用機会あり".into(), BeliefConfig::features_only()),
        ("相手傾向もあり(全部)".into(), BeliefConfig::full()),
    ];
    let mut with_hazard = true;
    let mut dump_lowconf = 0usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dump-lowconf" => {
                i += 1;
                dump_lowconf = args[i].parse().expect("数値");
            }
            "--games" => {
                i += 1;
                games = args[i].parse().expect("数値");
            }
            "--seed" => {
                i += 1;
                seed = args[i].parse().expect("数値");
            }
            "--bots" => {
                i += 1;
                names = args[i].split(',').map(|s| s.to_string()).collect();
            }
            "--particles" => {
                i += 1;
                particles = args[i].parse().expect("数値");
            }
            "--only-full" => {
                cfgs = vec![("相手傾向もあり(全部)".into(), BeliefConfig::full())];
            }
            "--no-hazard" => with_hazard = false,
            other => panic!("知らない引数: {other}"),
        }
        i += 1;
    }
    for (_, c) in cfgs.iter_mut() {
        c.particles = particles;
    }
    let n = names.len();
    assert!((3..=4).contains(&n));
    println!("校正: {} / {games} 局 / seed {seed} / 粒子 {particles}", names.join(","));
    let mut stats: Vec<Stats> = cfgs.iter().map(|_| Stats::default()).collect();
    let perms = permutations(n);
    let cfg_game = GameConfig { turn_time_limit_ms: None, ..GameConfig::default() };
    let t0 = std::time::Instant::now();
    for gi in 0..games {
        let gseed = seed + gi as u64;
        let mut bots: Vec<Box<dyn Bot>> = names.iter().enumerate().map(|(k, nm)| make_bot(nm, gseed ^ ((k as u64 + 1) << 32)).expect("知らないボット")).collect();
        let bot_at_seat = &perms[gi % perms.len()];
        for (b, bot) in bots.iter_mut().enumerate() {
            bot.reset(gseed ^ ((b as u64 + 1) << 32));
        }
        let mut g = Game::with_config(n as u8, gseed, cfg_game);
        // 席 × 設定 の受動的な Belief
        let mut beliefs: Vec<Vec<Belief>> = cfgs
            .iter()
            .map(|(_, c)| (0..n).map(|v| Belief::new(v as PlayerId, n, *c, gseed * 16 + v as u64)).collect())
            .collect();
        // ハザードの正解待ち: (設定, 予測index, プレイヤー, 予測を出した手番番号)
        let mut pending: Vec<(usize, usize, PlayerId, u32)> = Vec::new();
        let mut seq = 0u64;
        let mut buf = Vec::new();
        let mut actions = 0usize;
        while !g.is_over() && actions < 100_000 && g.turn < 400 {
            g.legal_actions_into(&mut buf);
            let seat = g.to_act as usize;
            let a = bots[bot_at_seat[seat]].decide(&View::new(&g, seat as PlayerId), &buf);
            let before = g.clone();
            let rec = g.apply(a);
            seq += 1;
            actions += 1;
            if bots.iter().any(|b| b.wants_events()) {
                catan_ai::harness::notify_bots(&mut bots, bot_at_seat, &before, &g, &rec, seq);
            }
            for (ci, bs) in beliefs.iter_mut().enumerate() {
                for b in bs.iter_mut() {
                    let ev = project_event(&before, &g, &rec, b.viewer, LEGACY_APP, seq);
                    let t = std::time::Instant::now();
                    b.observe(&ev);
                    stats[ci].update_ns += t.elapsed().as_nanos();
                    stats[ci].events += 1;
                }
            }
            // 手番終了時に採点
            if matches!(rec.action, Action::EndTurn) && !g.is_over() {
                let ended = rec.actor;
                // ハザードの正解: 予測時の相手 p の「次の手番」が終わった（p の EndTurn）か、p が勝った
                pending.retain(|&(ci, pi, p, turn_at)| {
                    if p == ended && g.turn > turn_at {
                        stats[ci].hazard[pi].truth = false;
                        false
                    } else {
                        true
                    }
                });
                for (ci, bs) in beliefs.iter().enumerate() {
                    for b in bs.iter() {
                        let me = b.viewer;
                        let obs = View::new(&g, me).observe(&[], seq);
                        stats[ci].ess.push(b.ess());
                        for q in 0..n {
                            let p = q as PlayerId;
                            if p == me {
                                continue;
                            }
                            let truth_vp = g.players[q].dev[DevCard::VictoryPoint.idx()];
                            if g.players[q].dev_count() > 0 {
                                let dist = b.vp_dist(p);
                                if dump_lowconf > 0 && truth_vp >= 1 && 1.0 - dist[0] < 0.1 {
                                    dump_lowconf -= 1;
                                    let deck_vp: Vec<u8> = b.dev.iter().filter(|pt| pt.w > 0.0).take(3).map(|pt| pt.deck[DevCard::VictoryPoint.idx()]).collect();
                                    let held: Vec<[u8; 5]> = b.dev.iter().filter(|pt| pt.w > 0.0).take(2).map(|pt| pt.dev(p)).collect();
                                    println!(
                                        "[低確信] cfg={ci} 局{gi} 手番{} viewer={me} p={p} 公開VP={:?} 真dev={:?} 真山={:?} 予測dist={:?} 粒子の山VP={:?} 粒子の p の札={:?} 台帳購入={:?} 使用={:?} own_dev={:?} 全滅={} ESS={:.0}",
                                        g.turn,
                                        (0..n).map(|r| g.public_vp(r as u8)).collect::<Vec<_>>(),
                                        g.players[q].dev,
                                        g.dev_deck,
                                        dist,
                                        deck_vp,
                                        held,
                                        b.hist.ledgers[q].purchases,
                                        b.hist.ledgers[q].plays,
                                        g.players[me as usize].dev,
                                        b.diag.collapses(),
                                        b.ess()
                                    );
                                }
                                stats[ci].vp.push(VpPred { dist, truth: truth_vp, dev_count: g.players[q].dev_count() });
                            }
                            if g.players[q].hand_size() > 0 {
                                stats[ci].hand_hit.push(b.prob_hand(p, &g.players[q].hand));
                                let e = b.expected_hand(p);
                                let mae: f32 = (0..NUM_RESOURCES).map(|r| (e[r] - g.players[q].hand[r] as f32).abs()).sum::<f32>() / NUM_RESOURCES as f32;
                                stats[ci].hand_mae.push(mae);
                            }
                            if with_hazard && g.public_vp(p) >= 6 {
                                let t = std::time::Instant::now();
                                let bank = obs.public.bank.unwrap_or([0; NUM_RESOURCES]);
                                let mut acc = 0.0f32;
                                for (hand, dev, deck, others, w) in b.distinct_states(p) {
                                    let mut s = ReachSolver::new(&obs.public.game, p, dev, deck, others, bank);
                                    acc += w * s.win_prob_next_turn(&hand);
                                }
                                stats[ci].hazard_ns += t.elapsed().as_nanos();
                                stats[ci].hazard_calls += 1;
                                stats[ci].hazard.push(HazardPred { p: acc, truth: false });
                                pending.push((ci, stats[ci].hazard.len() - 1, p, g.turn));
                            }
                        }
                    }
                }
            }
        }
        // 終局: 勝者の手番で終わった予測は「勝った」
        if let Some(w) = g.winner {
            for &(ci, pi, p, _) in &pending {
                if p == w {
                    stats[ci].hazard[pi].truth = true;
                }
            }
        }
        for (ci, bs) in beliefs.iter().enumerate() {
            for b in bs {
                stats[ci].collapses += b.diag.collapses();
                stats[ci].inconsistent += b.diag.inconsistent;
                stats[ci].resyncs += b.diag.resyncs;
            }
        }
    }
    println!("所要 {:.1} 秒\n", t0.elapsed().as_secs_f64());
    for (ci, (label, _)) in cfgs.iter().enumerate() {
        println!("{}", report_calibration(label, &stats[ci]));
    }
}

/// 相手モデルの当てはめ: 対照 CPU の対局で「使えた札を使ったか」を数える（採点器）
pub fn fit(args: &[String], make_bot: &dyn Fn(&str, u64) -> Option<Box<dyn Bot>>) {
    let mut games = 200usize;
    let mut seed = 6_000_000u64;
    let mut names: Vec<String> = vec!["v2_control".into(); 4];
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--games" => {
                i += 1;
                games = args[i].parse().expect("数値");
            }
            "--seed" => {
                i += 1;
                seed = args[i].parse().expect("数値");
            }
            "--bots" => {
                i += 1;
                names = args[i].split(',').map(|s| s.to_string()).collect();
            }
            other => panic!("知らない引数: {other}"),
        }
        i += 1;
    }
    let n = names.len();
    println!("当てはめ: {} / {games} 局 / seed {seed}", names.join(","));
    // [種類][特徴バケット] = (機会, 使用)
    // バケット: 騎士 0=普通 1=盗賊に塞がれ 2=騎士力が取れる 3=両方 / 街道 0=普通 1=最長路レース / 収穫 0 / 独占 0..3=他人の手札枚数 <6,<12,<18,≥18
    let mut cnt = [[(0u32, 0u32); 4]; NUM_DEV_KINDS];
    // 保持した自分の手番数（この手番を 1 とする）別: [種類][min(手番数,5)] = (機会, 使用)
    let mut cnt_dur = [[(0u32, 0u32); 6]; NUM_DEV_KINDS];
    let mut hold_turns: Vec<(DevCard, u32, bool)> = Vec::new(); // (種類, 保持した自分の手番数, 最後まで使わなかったか)
    let perms = permutations(n);
    let cfg_game = GameConfig { turn_time_limit_ms: None, ..GameConfig::default() };
    let mut p_default = OpponentParams::default();
    p_default.use_styles = false;
    for gi in 0..games {
        let gseed = seed + gi as u64;
        let mut bots: Vec<Box<dyn Bot>> = names.iter().enumerate().map(|(k, nm)| make_bot(nm, gseed ^ ((k as u64 + 1) << 32)).expect("知らないボット")).collect();
        let bot_at_seat = &perms[gi % perms.len()];
        for (b, bot) in bots.iter_mut().enumerate() {
            bot.reset(gseed ^ ((b as u64 + 1) << 32));
        }
        let mut g = Game::with_config(n as u8, gseed, cfg_game);
        let mut seq = 0u64;
        let mut buf = Vec::new();
        let mut actions = 0usize;
        // 手番の開始時点の使用機会
        let mut turn_start: Option<(PlayerId, Opportunity, [u8; NUM_DEV_KINDS])> = None;
        let mut played_kind: Option<DevCard> = None;
        // 各札の保持: (player, kind, bought_turn_count) → 使われたら記録
        let mut holdings: Vec<(PlayerId, DevCard, u32)> = Vec::new();
        let mut turns_ended = [0u32; 4];
        while !g.is_over() && actions < 100_000 && g.turn < 400 {
            g.legal_actions_into(&mut buf);
            let seat = g.to_act as usize;
            let a = bots[bot_at_seat[seat]].decide(&View::new(&g, seat as PlayerId), &buf);
            let before = g.clone();
            let rec = g.apply(a);
            seq += 1;
            actions += 1;
            if bots.iter().any(|b| b.wants_events()) {
                catan_ai::harness::notify_bots(&mut bots, bot_at_seat, &before, &g, &rec, seq);
            }
            let ev = project_event(&before, &g, &rec, 0, LEGACY_APP, seq);
            let actor = rec.actor;
            if !before.is_setup() && actor == before.turn_player {
                if turn_start.map_or(true, |(p, _, _)| p != actor) {
                    // 手番の開始: 使える札（今手番に買った分を除く）
                    let mut playable = [0u8; NUM_DEV_KINDS];
                    for k in 0..NUM_DEV_KINDS {
                        playable[k] = before.players[actor as usize].dev[k] - before.players[actor as usize].dev_bought_this_turn[k];
                    }
                    turn_start = Some((actor, Opportunity::from_context(&ev.before, actor, n), playable));
                    played_kind = None;
                }
            }
            match ev.payload {
                VisibleEvent::BuyDev { .. } => {
                    if let catan_core::action::Outcome::Drew(c) = rec.result {
                        holdings.push((actor, c, turns_ended[actor as usize]));
                    }
                }
                VisibleEvent::PlayKnight => played_kind = Some(DevCard::Knight),
                VisibleEvent::PlayRoadBuilding => played_kind = Some(DevCard::RoadBuilding),
                VisibleEvent::PlayYearOfPlenty { .. } => played_kind = Some(DevCard::YearOfPlenty),
                VisibleEvent::PlayMonopoly { .. } => played_kind = Some(DevCard::Monopoly),
                VisibleEvent::EndTurn => {
                    if let Some((p, opp, playable)) = turn_start.take() {
                        if p == actor {
                            let end_opp = Opportunity::from_context(&ev.before, actor, n);
                            let x = Opportunity {
                                blocked_pips: opp.blocked_pips.max(end_opp.blocked_pips),
                                army_gain: opp.army_gain || end_opp.army_gain,
                                can_build_road: opp.can_build_road || end_opp.can_build_road,
                                road_race: opp.road_race || end_opp.road_race,
                                bank_has_two: opp.bank_has_two || end_opp.bank_has_two,
                                ..end_opp
                            };
                            for c in DEV_CARDS {
                                if c == DevCard::VictoryPoint || playable[c.idx()] == 0 {
                                    continue;
                                }
                                let bucket = match c {
                                    DevCard::Knight => (x.blocked_pips > 0) as usize + 2 * x.army_gain as usize,
                                    DevCard::RoadBuilding => {
                                        if !x.can_build_road {
                                            continue;
                                        }
                                        x.road_race as usize
                                    }
                                    DevCard::YearOfPlenty => {
                                        if !x.bank_has_two {
                                            continue;
                                        }
                                        0
                                    }
                                    DevCard::Monopoly => (x.others_hand_total / 6).min(3) as usize,
                                    _ => 0,
                                };
                                cnt[c.idx()][bucket].0 += 1;
                                if played_kind == Some(c) {
                                    cnt[c.idx()][bucket].1 += 1;
                                }
                                // 最も古い同種の札の保持手番数（買った手番の次の手番を 1 とする）
                                let oldest = holdings.iter().filter(|h| h.0 == actor && h.1 == c).map(|h| turns_ended[actor as usize] - h.2).max().unwrap_or(0);
                                let d = (oldest as usize).min(5);
                                cnt_dur[c.idx()][d].0 += 1;
                                if played_kind == Some(c) {
                                    cnt_dur[c.idx()][d].1 += 1;
                                }
                            }
                            if let Some(c) = played_kind {
                                // 使った札の保持期間（最も古い同種の札とみなす）
                                if let Some(pos) = holdings.iter().position(|h| h.0 == actor && h.1 == c) {
                                    let (_, _, bt) = holdings.remove(pos);
                                    hold_turns.push((c, turns_ended[actor as usize] - bt, false));
                                }
                            }
                        }
                    }
                    turns_ended[actor as usize] += 1;
                }
                _ => {}
            }
        }
        for (p, c, bt) in holdings {
            hold_turns.push((c, turns_ended[p as usize].saturating_sub(bt), true));
        }
    }
    println!("種類 / 特徴バケット: 機会 / 使用 / 使用率 / 現在のモデル q（標準傾向）");
    let names_b: [&[&str]; 5] = [
        &["普通", "盗賊に塞がれ", "騎士力が取れる", "両方"],
        &["普通", "最長路レース", "", ""],
        &["普通", "", "", ""],
        &["他人の手札<6", "<12", "<18", "≥18"],
        &["", "", "", ""],
    ];
    for c in DEV_CARDS {
        if c == DevCard::VictoryPoint {
            continue;
        }
        for b in 0..4 {
            let (m, u) = cnt[c.idx()][b];
            if m == 0 {
                continue;
            }
            let x = match c {
                DevCard::Knight => Opportunity { blocked_pips: if b & 1 == 1 { 5 } else { 0 }, army_gain: b & 2 == 2, ..Default::default() },
                DevCard::RoadBuilding => Opportunity { can_build_road: true, road_race: b == 1, ..Default::default() },
                DevCard::YearOfPlenty => Opportunity { bank_has_two: true, ..Default::default() },
                DevCard::Monopoly => Opportunity { others_hand_total: (b as u8) * 6 + 3, ..Default::default() },
                _ => Opportunity::default(),
            };
            println!(
                "  {:<6} {:<12} {:>6} {:>6} {:.3}  q={:.3}",
                c.ja(),
                names_b[c.idx()][b],
                m,
                u,
                u as f32 / m as f32,
                q_use(c, &x, STYLE_BALANCED, 1, &p_default)
            );
        }
    }
    println!("\n保持した自分の手番数（買った手番の次の手番 = 1）別の使用率: 種類 / 手番数 / 機会 / 使用 / 使用率");
    for c in DEV_CARDS {
        if c == DevCard::VictoryPoint {
            continue;
        }
        for d in 1..6 {
            let (m, u) = cnt_dur[c.idx()][d];
            if m > 0 {
                println!("  {:<6} {}{}  {:>6} {:>6} {:.3}", c.ja(), d, if d == 5 { "+" } else { " " }, m, u, u as f32 / m as f32);
            }
        }
    }
    println!("\n保持した自分の手番数（買った手番を 0 とする）の分布: 使った札 / 最後まで持った札");
    for c in DEV_CARDS {
        let used: Vec<u32> = hold_turns.iter().filter(|h| h.0 == c && !h.2).map(|h| h.1).collect();
        let kept: Vec<u32> = hold_turns.iter().filter(|h| h.0 == c && h.2).map(|h| h.1).collect();
        let mean = |v: &Vec<u32>| if v.is_empty() { 0.0 } else { v.iter().sum::<u32>() as f32 / v.len() as f32 };
        println!("  {:<6} 使った {} 枚（平均 {:.2} 手番後）/ 最後まで {} 枚（平均 {:.2} 手番保持）", c.ja(), used.len(), mean(&used), kept.len(), mean(&kept));
    }
}

/// 1 局を v2 で指させ、判断の説明を数か所出す（説明出力の確認用）
pub fn explain(args: &[String]) {
    let mut seed = 7_000_000u64;
    let mut every = 40usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => {
                i += 1;
                seed = args[i].parse().expect("数値");
            }
            "--every" => {
                i += 1;
                every = args[i].parse().expect("数値");
            }
            other => panic!("知らない引数: {other}"),
        }
        i += 1;
    }
    let mut v2 = AgentV2Bot::new(seed, V2Config::default(), "v2");
    let mut others: Vec<Box<dyn Bot>> = (1..4).map(|k| catan_ai::bots::bot_for_level(3, seed + k) as Box<dyn Bot>).collect();
    let cfg_game = GameConfig { turn_time_limit_ms: None, ..GameConfig::default() };
    let mut g = Game::with_config(4, seed, cfg_game);
    let mut buf = Vec::new();
    let mut seq = 0u64;
    let mut k = 0usize;
    while !g.is_over() && g.turn < 400 {
        g.legal_actions_into(&mut buf);
        let seat = g.to_act as usize;
        let a = if seat == 0 {
            let a = v2.decide(&View::new(&g, 0), &buf);
            k += 1;
            if k % every == 0 && !g.is_setup() {
                println!("--- 手番 {} P0 の判断（公開点 {:?}）", g.turn, (0..4).map(|q| g.public_vp(q)).collect::<Vec<_>>());
                println!("{}", v2.explain());
            }
            a
        } else {
            others[seat - 1].decide(&View::new(&g, seat as u8), &buf)
        };
        let before = g.clone();
        let rec = g.apply(a);
        seq += 1;
        let ev = project_event(&before, &g, &rec, 0, LEGACY_APP, seq);
        v2.observe(&ev);
    }
    println!("終局: 勝者 {:?} / 手番 {} / 実際の勝利点 {:?}", g.winner, g.turn, (0..4).map(|q| g.actual_vp(q)).collect::<Vec<_>>());
}
