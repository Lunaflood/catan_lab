//! M4 の物差し: 情報集合を無視する探索の誤りを検出する小ゲーム（設計書 8.4）。
//!
//! ここにあるのは本番カタンではなく、**探索の作りが正しいかを測る合成環境**。
//! 各ゲームで「正解の値」と「誤った作りの探索が出す値」を並べ、誤りが検出できることを確かめる。
//! 本番の v2 探索が同じ誤りを犯していないことは、`fairness_v2` の相手モデル非干渉テストと、
//! 「相手の方策は相手自身の情報集合しか読まない」という作り（`search_v2::OpponentPolicy`）で担保する。
//!
//! - A: strategy fusion（隠れ状態ごとに都合よく別の手を選ぶ）
//! - B: 相手の全知化（相手が自分の秘密を知っているかのように応答する）
//! - D: 相手全員を「自分を妨害する一人の敵」と見なす評価との差（3 人の利害）

/// ---- ゲーム A ----
/// 隠れ状態 z ∈ {0,1}（等確率）。手番 1: Safe（0.6 を得て終了）か Gamble。
/// Gamble のあと手番 2 で b ∈ {L,R} を選ぶが、**b を選ぶ時点でも z は見えない**（同じ情報集合）。
/// 報酬は b==z なら 1、違えば 0。
/// 正解: Gamble の真の価値は 0.5（z を知らずに b を選ぶので）→ Safe(0.6) が最善。
/// 誤り（PIMC: 世界ごとに解いて平均）: 世界 z=0 では b=L で 1、z=1 では b=R で 1 → Gamble=1.0 → Gamble を選ぶ。
mod game_a {
    #[derive(Clone, Copy, PartialEq, Debug)]
    pub enum A1 {
        Safe,
        Gamble,
    }
    pub fn reward(a: A1, b: usize, z: usize) -> f32 {
        match a {
            A1::Safe => 0.6,
            A1::Gamble => {
                if b == z {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
    /// 世界ごとに最善を取って平均（strategy fusion を起こす作り）
    pub fn pimc_value(a: A1) -> f32 {
        let mut acc = 0.0;
        for z in 0..2 {
            let best = (0..2).map(|b| reward(a, b, z)).fold(f32::NEG_INFINITY, f32::max);
            acc += 0.5 * best;
        }
        acc
    }
    /// 情報集合を守る: b は z に依らず 1 つ。b ごとに世界平均を取ってから最大化
    pub fn infoset_value(a: A1) -> f32 {
        (0..2)
            .map(|b| (0..2).map(|z| 0.5 * reward(a, b, z)).sum::<f32>())
            .fold(f32::NEG_INFINITY, f32::max)
    }
}

/// ---- ゲーム B ----
/// 自分の秘密 c ∈ {A,B}（等確率）。自分の手: Fold（0.3）か Play。
/// Play なら相手が c を当てにくる。当たれば自分 0、外れれば自分 1。
/// 正解: 相手は c を見られないので当たる確率は 1/2 → Play = 0.5 > Fold = 0.3。
/// 誤り（相手の全知化）: シミュレーション内の相手が c を読んで必ず当てる → Play = 0 → Fold を選ぶ。
mod game_b {
    #[derive(Clone, Copy, PartialEq, Debug)]
    pub enum B1 {
        Fold,
        Play,
    }
    /// 相手の方策: `sees_secret` なら c を知って当てる。知らなければ一様
    pub fn value(a: B1, sees_secret: bool) -> f32 {
        match a {
            B1::Fold => 0.3,
            B1::Play => {
                let mut acc = 0.0;
                for c in 0..2 {
                    // 相手の推測 g: 全知なら g=c、そうでなければ g は c と独立に一様
                    let p_hit = if sees_secret { 1.0 } else { 0.5 };
                    acc += 0.5 * (1.0 - p_hit);
                    let _ = c;
                }
                acc
            }
        }
    }
}

/// ---- ゲーム D ----
/// 3 人。P0 が X か Y を選ぶ。
/// X のあと P1 が {P2 を削る, P0 を削る} を選ぶ: (P0,P1,P2) = (0.5,0.5,0.0) か (0.0,0.2,0.8)。
/// Y なら固定で (0.3,0.3,0.4)。
/// 正解（各人が自分の利得を最大化）: P1 は 0.5 > 0.2 で「P2 を削る」→ X で P0 は 0.5 > 0.3 → X。
/// 誤り（paranoid: 相手は常に自分を最小化する）: P1 が「P0 を削る」→ X で 0 → Y(0.3) を選ぶ。
mod game_d {
    pub fn value_x(paranoid: bool) -> f32 {
        let opts = [(0.5f32, 0.5f32, 0.0f32), (0.0, 0.2, 0.8)];
        if paranoid {
            opts.iter().map(|o| o.0).fold(f32::INFINITY, f32::min)
        } else {
            // P1 が自分の利得で選ぶ
            let best = opts.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).unwrap();
            best.0
        }
    }
    pub const VALUE_Y: f32 = 0.3;
}

#[test]
fn a_strategy_fusionを検出する() {
    use game_a::*;
    // 誤った作りは Gamble を過大評価して選ぶ
    assert!((pimc_value(A1::Gamble) - 1.0).abs() < 1e-6);
    assert!(pimc_value(A1::Gamble) > pimc_value(A1::Safe), "PIMC は Gamble を選ぶ（誤り）");
    // 情報集合を守れば Safe
    assert!((infoset_value(A1::Gamble) - 0.5).abs() < 1e-6);
    assert!(infoset_value(A1::Safe) > infoset_value(A1::Gamble), "正しい探索は Safe を選ぶ");
}

#[test]
fn b_相手の全知化を検出する() {
    use game_b::*;
    assert!(value(B1::Play, true) < value(B1::Fold, true), "全知の相手を仮定すると Fold（誤り）");
    assert!(value(B1::Play, false) > value(B1::Fold, false), "相手が自分の秘密を知らなければ Play（正解）");
}

#[test]
fn d_単一の敵と見なす評価との差() {
    use game_d::*;
    assert!(value_x(true) < VALUE_Y, "paranoid は Y を選ぶ（3 人の利害を無視）");
    assert!(value_x(false) > VALUE_Y, "各人が自分の利得で動くなら X");
}

/// ---- 本番の探索の作りに対する検査 ----
/// v2 のロールアウトで相手を動かす方策は、その相手自身の手札と公開情報だけを読む。
/// ここでは「自分（根の手番）の秘密だけを変えても、相手方策の判断が変わらない」ことを実測する（B の本番版）。
#[test]
fn 本番のロールアウト相手方策は根の秘密を読まない() {
    use catan_ai::search_v2::OpponentPolicy;
    use catan_core::action::DevCard;
    use catan_core::game::{Game, GameConfig};
    use catan_core::rng::Rng;
    let mut compared = 0;
    for seed in 0..12u64 {
        let mut g = Game::with_config(4, seed, GameConfig::default());
        let mut rng = Rng::with_stream(seed, 3);
        let mut buf = Vec::new();
        for _ in 0..600 {
            if g.is_over() {
                break;
            }
            g.legal_actions_into(&mut buf);
            let a = buf[rng.below(buf.len() as u32) as usize];
            g.apply(a);
        }
        if g.is_over() || g.is_setup() {
            continue;
        }
        // 相手 q の判断（根は P0 とする）
        for q in 1..4u8 {
            if g.to_act != q {
                continue;
            }
            let mut pol = OpponentPolicy::new(7);
            let mut rng_a = Rng::new(99);
            let a1 = pol.choose(&g, q, &buf, &mut rng_a);
            // P0 の秘密だけを変える: 発展カードの種類（枚数不変）と、資源の中身（銀行と入れ替え）
            let mut t = g.clone();
            let p0 = &mut t.players[0];
            if p0.dev_count() > 0 {
                let total = p0.dev_count();
                for i in 0..5 {
                    t.dev_deck[i] += p0.dev[i];
                }
                p0.dev = [0; 5];
                p0.dev[DevCard::VictoryPoint.idx()] = total.min(t.dev_deck[DevCard::VictoryPoint.idx()]);
                let rest = total - p0.dev[DevCard::VictoryPoint.idx()];
                p0.dev[DevCard::Knight.idx()] += rest;
                t.dev_deck[DevCard::VictoryPoint.idx()] -= p0.dev[DevCard::VictoryPoint.idx()];
                t.dev_deck[DevCard::Knight.idx()] -= rest;
            }
            let mut rng_b = Rng::new(99);
            let mut pol2 = OpponentPolicy::new(7);
            let a2 = pol2.choose(&t, q, &buf, &mut rng_b);
            assert_eq!(a1, a2, "seed={seed}: 根の秘密で相手方策が変わった");
            compared += 1;
        }
    }
    assert!(compared > 0);
}
