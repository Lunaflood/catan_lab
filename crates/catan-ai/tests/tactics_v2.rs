//! M3: 戦術局面集（設計書 10）。正解は手で導く。「旧 CPU が選んだ手」を正解にしない。
//!
//! 各局面で、隠れ勝利点の分布と勝利ハザードが判断を**どう変えるか**を、
//! ハザードあり／なしの v2 を並べて示す。

use catan_ai::agent_v2::{AgentV2, V2Config};
use catan_ai::belief::BeliefConfig;
use catan_core::action::{Action, DevCard, Prompt};
use catan_core::board::{Building, BuildingKind};
use catan_core::game::{Forced, Game, GameConfig};
use catan_core::observation::LEGACY_APP;
use catan_core::observer::{observe, project_event};
use catan_core::rng::Rng;
use catan_core::topology::{Topology, NUM_NODES};

const WOOD: usize = 0;
const BRICK: usize = 1;
const SHEEP: usize = 2;
const WHEAT: usize = 3;
const ORE: usize = 4;

/// 行動を適用して P0 のエージェント群に出来事を配る
fn step(g: &mut Game, a: Action, forced: Forced, agents: &mut [AgentV2], seq: &mut u64) {
    let before = g.clone();
    let rec = g.apply_forced(a, forced);
    *seq += 1;
    for ag in agents.iter_mut() {
        let ev = project_event(&before, g, &rec, 0, LEGACY_APP, *seq);
        ag.observe(&ev);
    }
}

/// 産出として資源を渡す（公開の出来事として配る）
fn grant(g: &mut Game, p: u8, bundle: [u8; 5], agents: &mut [AgentV2], seq: &mut u64) {
    let before = g.clone();
    for r in 0..5 {
        g.bank[r] -= bundle[r];
        g.players[p as usize].hand[r] += bundle[r];
    }
    *seq += 1;
    let rec = catan_core::action::ActionRecord { actor: p, action: Action::Roll, result: catan_core::action::Outcome::Dice(1, 1) };
    for ag in agents.iter_mut() {
        let ev = project_event(&before, g, &rec, 0, LEGACY_APP, *seq);
        ag.observe(&ev);
    }
}

/// 公開の廃棄として資源を銀行へ戻す（手札を減らす唯一の公開の手段）
fn discard_public(g: &mut Game, p: u8, bundle: [u8; 5], agents: &mut [AgentV2], seq: &mut u64) {
    let before = g.clone();
    for r in 0..5 {
        g.bank[r] += bundle[r];
        g.players[p as usize].hand[r] -= bundle[r];
    }
    *seq += 1;
    let rec = catan_core::action::ActionRecord { actor: p, action: Action::Discard(bundle), result: catan_core::action::Outcome::None };
    for ag in agents.iter_mut() {
        let ev = project_event(&before, g, &rec, 0, LEGACY_APP, *seq);
        ag.observe(&ev);
    }
}

/// 手札をちょうど `target` にする（余りは公開の廃棄、不足は産出として配る）
fn set_hand(g: &mut Game, p: u8, target: [u8; 5], agents: &mut [AgentV2], seq: &mut u64) {
    let h = g.players[p as usize].hand;
    let mut extra = [0u8; 5];
    let mut short = [0u8; 5];
    for r in 0..5 {
        extra[r] = h[r].saturating_sub(target[r]);
        short[r] = target[r].saturating_sub(h[r]);
    }
    if extra.iter().any(|&v| v > 0) {
        discard_public(g, p, extra, agents, seq);
    }
    if short.iter().any(|&v| v > 0) {
        grant(g, p, short, agents, seq);
    }
    assert_eq!(g.players[p as usize].hand, target);
}

fn ready(seed: u64, agents: &mut [AgentV2], seq: &mut u64) -> Game {
    let mut g = Game::with_config(4, seed, GameConfig { turn_time_limit_ms: None, ..GameConfig::default() });
    let mut rng = Rng::with_stream(seed, 5);
    let mut buf = Vec::new();
    while g.is_setup() {
        g.legal_actions_into(&mut buf);
        let a = buf[rng.below(buf.len() as u32) as usize];
        step(&mut g, a, Forced::No, agents, seq);
    }
    g
}

/// 公開点を `vp` にする（建物の置き直し。都市の駒を 1 つ残す）
fn set_public_vp(g: &mut Game, p: u8, vp: u8) {
    set_public_vp_avoiding(g, p, vp, None)
}

/// 同上。`avoid` の資源を産出する頂点を避けて置く（産出で届いてしまう局面を作らないため）
fn set_public_vp_avoiding(g: &mut Game, p: u8, vp: u8, avoid: Option<usize>) {
    let topo = Topology::get();
    let open = |g: &Game, n: usize| {
        g.board.building[n].is_none()
            && topo.node_neighbors[n].as_slice().iter().all(|&m| g.board.building[m as usize].is_none())
            && avoid.map_or(true, |r| g.board.node_pips(n as u8)[r] == 0)
    };
    let cities = (vp / 2).min(3);
    let settlements = vp - 2 * cities;
    for n in 0..NUM_NODES {
        if matches!(g.board.building[n], Some(b) if b.owner == p) {
            g.board.building[n] = None;
        }
    }
    // 道も消す（残っていると、その先の空き頂点に開拓地が建てられてしまい、局面の意図が変わる）
    for e in 0..catan_core::topology::NUM_EDGES {
        if g.board.road[e] == Some(p) {
            g.board.road[e] = None;
        }
    }
    g.players[p as usize].roads_left = 15;
    g.players[p as usize].longest_road = 0;
    g.players[p as usize].settlements_left = 5;
    g.players[p as usize].cities_left = 4;
    let mut placed = 0u8;
    let mut converted = 0u8;
    while placed < settlements + cities {
        if g.players[p as usize].settlements_left == 0 {
            let n = (0..NUM_NODES).find(|&n| matches!(g.board.building[n], Some(b) if b.owner == p && b.kind == BuildingKind::Settlement)).unwrap();
            g.board.building[n] = Some(Building { owner: p, kind: BuildingKind::City });
            g.players[p as usize].cities_left -= 1;
            g.players[p as usize].settlements_left += 1;
            converted += 1;
            continue;
        }
        let n = (0..NUM_NODES).find(|&n| open(g, n)).expect("空き頂点");
        g.board.building[n] = Some(Building { owner: p, kind: BuildingKind::Settlement });
        g.players[p as usize].settlements_left -= 1;
        placed += 1;
    }
    while converted < cities {
        let n = (0..NUM_NODES).find(|&n| matches!(g.board.building[n], Some(b) if b.owner == p && b.kind == BuildingKind::Settlement)).unwrap();
        g.board.building[n] = Some(Building { owner: p, kind: BuildingKind::City });
        g.players[p as usize].cities_left -= 1;
        g.players[p as usize].settlements_left += 1;
        converted += 1;
    }
    g.longest_road_owner = None;
    g.largest_army_owner = None;
    assert_eq!(g.public_vp(p), vp);
}

fn agents() -> Vec<AgentV2> {
    let b = BeliefConfig { particles: 512, ..BeliefConfig::features_only() };
    vec![
        AgentV2::new(11, V2Config { belief: b, use_hazard: true, ..V2Config::default() }),
        AgentV2::new(11, V2Config { belief: b, use_hazard: false, ..V2Config::default() }),
    ]
}

/// 局面: 相手 P1 は公開 8 点。1 枚の発展カードを 4 手番使わずに持っている（＝勝利点らしい）。
/// P1 が「木 1 を出すから麦 1 が欲しい」と提案。麦 1 が入ると P1 は都市が建つ（麦 2 鉄 3）＝ 10 点。
/// 自分（P0）にとって木は開拓地の最後の 1 枚で、資源だけ見れば得な交換。
/// 正解: 断る（受けると P1 が次の手番に高確率で勝つ）。
#[test]
fn 公開8点と隠れvpの分布が同じ交換の危険度を変える() {
    let mut ags = agents();
    let mut seq = 0;
    let mut g = ready(101, &mut ags, &mut seq);
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut ags, &mut seq);
    step(&mut g, Action::EndTurn, Forced::No, &mut ags, &mut seq);
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut ags, &mut seq);
    grant(&mut g, 1, [0, 0, 1, 1, 1], &mut ags, &mut seq);
    step(&mut g, Action::BuyDevCard, Forced::Draw(DevCard::VictoryPoint), &mut ags, &mut seq);
    step(&mut g, Action::EndTurn, Forced::No, &mut ags, &mut seq);
    while g.turn_player != 0 {
        step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut ags, &mut seq);
        step(&mut g, Action::EndTurn, Forced::No, &mut ags, &mut seq);
    }
    for _ in 0..4 {
        for _ in 0..4 {
            step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut ags, &mut seq);
            step(&mut g, Action::EndTurn, Forced::No, &mut ags, &mut seq);
        }
    }
    // P1 は麦を産出しない場所に置く（産出で麦が届くと、交換の前から危険になってしまう）
    set_public_vp_avoiding(&mut g, 1, 8, Some(WHEAT));
    // P0 の手番を終え、P1 の手番で P1 が提案する
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut ags, &mut seq);
    step(&mut g, Action::EndTurn, Forced::No, &mut ags, &mut seq);
    assert_eq!(g.turn_player, 1);
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut ags, &mut seq);
    // 出目の産出の後で手札を整える（公開の廃棄と産出として配るので、推定の履歴は保たれる）
    // P1: 木 1 麦 1 鉄 3（麦があと 1 枚で都市）
    set_hand(&mut g, 1, [1, 0, 0, 1, 3], &mut ags, &mut seq);
    // P0: 木 0 土 1 羊 1 麦 3 鉄 3。麦 1 を出して木 1 を貰えば、都市・発展に加えて開拓地も払える
    // （資源だけ見れば得な交換）
    set_hand(&mut g, 0, [0, 1, 1, 3, 3], &mut ags, &mut seq);
    let give = [1, 0, 0, 0, 0];
    let want = [0, 0, 0, 1, 0];
    assert!(g.can_apply(&Action::OfferTrade { give, want }), "P1 は木を持っている");
    step(&mut g, Action::OfferTrade { give, want }, Forced::No, &mut ags, &mut seq);
    // P2, P3 は断る
    step(&mut g, Action::RejectTrade, Forced::No, &mut ags, &mut seq);
    step(&mut g, Action::RejectTrade, Forced::No, &mut ags, &mut seq);
    assert_eq!(g.to_act, 0);
    assert_eq!(g.prompt, Prompt::DecideTrade);
    let legal = g.legal_actions();
    assert!(legal.contains(&Action::AcceptTrade), "P0 は麦を持っている");
    let obs = observe(&g, 0, &legal, LEGACY_APP, seq);
    let with = ags[0].decide(&obs);
    let without = ags[1].decide(&obs);
    println!("ハザードあり: {with:?}\n{}", ags[0].explain());
    println!("ハザードなし: {without:?}\n{}", ags[1].explain());
    // 根拠: P1 の隠れ VP は高確率で 1（4 機会の不使用）。鉄が渡れば都市で 10 点
    let b = ags[0].belief.as_ref().unwrap();
    let p_vp = 1.0 - b.vp_dist(1)[0];
    assert!(p_vp > 0.6, "4 機会の不使用の後の P1 の隠れ VP 確率が低い: {p_vp}");
    assert!(ags[0].last.hazard[1] < 0.5, "交換前のハザードは低いはず（麦が足りない）: {}", ags[0].last.hazard[1]);
    assert_ne!(with, Action::AcceptTrade, "ハザードを見る v2 は P1 を勝たせる交換を受けない");
    let val = |ag: &AgentV2, a: Action| ag.last.top.iter().find(|v| v.action == a).map(|v| v.value);
    let (acc_w, rej_w) = (val(&ags[0], Action::AcceptTrade), val(&ags[0], Action::RejectTrade));
    let (acc_n, rej_n) = (val(&ags[1], Action::AcceptTrade), val(&ags[1], Action::RejectTrade));
    // 交換自体は資源だけ見れば得（ハザード無しでは受ける方が高い）。そうでなければ、この局面は判断の差を示さない
    assert!(acc_n.unwrap_or(f32::NEG_INFINITY) > rej_n.unwrap_or(f32::INFINITY), "ハザード無しでは受ける方が高いはず: 受 {acc_n:?} 断 {rej_n:?}");
    assert!(acc_w.unwrap_or(f32::INFINITY) < rej_w.unwrap_or(f32::NEG_INFINITY), "ハザードありでは受ける方が低いはず: 受 {acc_w:?} 断 {rej_w:?}");
}

/// 局面: 公開 9 点の自分。麦 2 鉄 2 木 4。一見損な 4:1 交換（木 4 → 鉄 1）を挟むと都市で 10 点。
/// 正解: 交換してから都市（この手番に勝つ）。旧探索でも 2 手読みで到達できるが、v2 でも失わないことを確かめる
#[test]
fn 一見損な銀行交換を挟むと即勝利() {
    let mut ags = agents();
    let mut seq = 0;
    let mut g = ready(103, &mut ags, &mut seq);
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut ags, &mut seq);
    set_public_vp(&mut g, 0, 9);
    let h = g.players[0].hand;
    let need = [4u8.saturating_sub(h[WOOD]), 0, 0, 2u8.saturating_sub(h[WHEAT]), 2u8.saturating_sub(h[ORE])];
    grant(&mut g, 0, need, &mut ags, &mut seq);
    if g.board.best_maritime_rate(0, catan_core::board::Resource::Wood) < 4 {
        return; // 港があると別の解になるので、この盤では検査しない
    }
    let mut steps = 0;
    while !g.is_over() && g.turn_player == 0 && steps < 8 {
        let legal = g.legal_actions();
        let obs = observe(&g, 0, &legal, LEGACY_APP, seq);
        let a = ags[0].decide(&obs);
        step(&mut g, a, Forced::No, &mut ags, &mut seq);
        steps += 1;
    }
    assert_eq!(g.winner, Some(0), "交換→都市で勝てる手番を逃した");
}

/// 局面: 盗賊の置き先。P1 は公開 8 点で隠れ VP らしい札を持ち、都市まであと少し。P2 は 4 点で産出は高い。
/// 正解: 産出の総量ではなく、次の手番に勝ちそうな P1 を止める
#[test]
fn 盗賊は勝ちそうな相手を止める() {
    let mut ags = agents();
    let mut seq = 0;
    let mut g = ready(107, &mut ags, &mut seq);
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut ags, &mut seq);
    step(&mut g, Action::EndTurn, Forced::No, &mut ags, &mut seq);
    step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut ags, &mut seq);
    grant(&mut g, 1, [0, 0, 1, 1, 1], &mut ags, &mut seq);
    step(&mut g, Action::BuyDevCard, Forced::Draw(DevCard::VictoryPoint), &mut ags, &mut seq);
    step(&mut g, Action::EndTurn, Forced::No, &mut ags, &mut seq);
    while g.turn_player != 0 {
        step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut ags, &mut seq);
        step(&mut g, Action::EndTurn, Forced::No, &mut ags, &mut seq);
    }
    for _ in 0..4 {
        for _ in 0..4 {
            step(&mut g, Action::Roll, Forced::Dice(1, 1), &mut ags, &mut seq);
            step(&mut g, Action::EndTurn, Forced::No, &mut ags, &mut seq);
        }
    }
    assert_eq!(g.turn_player, 0);
    set_public_vp(&mut g, 1, 8);
    set_public_vp(&mut g, 2, 4);
    // P1: 麦 2 鉄 2（鉄があと 1 枚で都市）。手札が多いので盗む価値もある
    let h1 = g.players[1].hand;
    grant(&mut g, 1, [0, 0, 0, 2u8.saturating_sub(h1[WHEAT]), 2u8.saturating_sub(h1[ORE])], &mut ags, &mut seq);
    // P0 が 7 を出して盗賊を動かす
    step(&mut g, Action::Roll, Forced::Dice(3, 4), &mut ags, &mut seq);
    // 廃棄が要る人が居れば処理（合法手の先頭）
    while g.prompt == Prompt::Discard {
        let a = g.legal_actions()[0];
        step(&mut g, a, Forced::No, &mut ags, &mut seq);
    }
    assert_eq!(g.prompt, Prompt::MoveRobber);
    let legal = g.legal_actions();
    let obs = observe(&g, 0, &legal, LEGACY_APP, seq);
    let with = ags[0].decide(&obs);
    println!("ハザードあり: {with:?}\n{}", ags[0].explain());
    let Action::MoveRobber { tile, victim } = with else { panic!("盗賊の手でない") };
    let topo = Topology::get();
    let hits_p1 = topo.tile_nodes[tile as usize].iter().any(|&n| matches!(g.board.building[n as usize], Some(b) if b.owner == 1));
    assert!(ags[0].last.hazard[1] > 0.0, "P1 のハザードが 0");
    assert!(hits_p1, "勝ちそうな P1 のヘクスに置いていない: {with:?}");
    assert_eq!(victim, Some(1), "奪う相手は P1");
    let _ = (SHEEP, BRICK);
}
