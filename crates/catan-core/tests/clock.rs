//! 時計が**規則に触れない**ことの見張り。
//!
//! オンラインは「合法手の何番目か」だけを配って各端末が盤を作り直す方式なので、
//! 手の並びや個数が端末ごとに違ってはいけない。
//! ところが `negotiation_open()` は `legal_actions`（game.rs:581, 671）と
//! `can_apply`（game.rs:1001, 1012）の両方に効いていて、
//! これは `turn_time_limit_ms` ＝ 時計に繋がっている。
//!
//! いまは 3 か所すべてで `None` なので門は定数で開いているが、
//! 「残り時間が尽きたら提案を出さない」のような**自然に見える改良**を
//! 1 行足した瞬間に、サーバ（仮想時計）と端末（実時間）で手の個数が割れる。
//! この試験はその 1 行を落とすために置いてある。

use catan_core::action::Action;
use catan_core::game::{Game, GameConfig, TurnClock};
use catan_core::rng::Rng;

/// UI と同じ設定（実時間の時計・持ち時間なし）で作る
fn ui_game(seed: u64) -> Game {
    let cfg = GameConfig {
        turn_clock: TurnClock::Realtime,
        turn_time_limit_ms: None,
        ..GameConfig::default()
    };
    Game::with_streams(4, seed, seed ^ 0xA1, seed ^ 0xB2, seed ^ 0xC3, cfg)
}

#[test]
fn 時計を進めても合法手も指紋も変わらない() {
    let mut checked = 0usize;
    for seed in 0..12u64 {
        let mut g = ui_game(700_000 + seed);
        let mut rng = Rng::with_stream(seed, 31);
        let mut buf: Vec<Action> = Vec::new();
        for _ in 0..600 {
            if g.is_over() {
                break;
            }
            g.legal_actions_into(&mut buf);
            if buf.is_empty() {
                break;
            }
            // いまの局面を控える
            let before_acts = buf.clone();
            let before_fp = g.fingerprint();

            // 実時間を大量に流し込む（10 分ぶん）
            let mut probe = g.clone();
            probe.tick(600_000);
            let after_acts = probe.legal_actions();
            let after_fp = probe.fingerprint();

            assert_eq!(
                before_acts, after_acts,
                "seed {seed}: 時計を進めたら合法手が変わった\
                 （時間が legal_actions に漏れている）"
            );
            assert_eq!(
                before_fp, after_fp,
                "seed {seed}: 時計を進めたら指紋が変わった\
                 （指紋に時間が混ざっている＝オンラインで毎手ズレ報告が出る）"
            );
            checked += 1;

            let a = buf[rng.below(buf.len() as u32) as usize];
            g.apply(a);
        }
    }
    println!("局面 {checked} 個で確認");
    assert!(checked > 2000);
}

/// 端末が使う設定では、交渉の門が**時間に関係なく**開いていること。
/// ここが閉じる作りになった瞬間、サーバと端末で手の個数が割れる
#[test]
fn 端末の設定では交渉の門が時間で閉じない() {
    let mut g = ui_game(4242);
    assert!(g.negotiation_open());
    g.tick(60 * 60 * 1000); // 1 時間
    assert!(
        g.negotiation_open(),
        "turn_time_limit_ms が None なのに時間で閉じた"
    );
    assert_eq!(g.turn_time_left_ms(), None);
}
