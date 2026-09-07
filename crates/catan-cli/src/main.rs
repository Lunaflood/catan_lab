//! 自動対戦と速度測定。
//!
//!   catan bench [試合数]              エンジンの速度を測る
//!   catan match <bot> <bot> <bot> [--games N] [--seed S] [--no-trade]
//!
//! ボット名: random / weighted / vpgreedy

use catan_ai::agent_v2::{AgentV2Bot, V2Config};
use catan_ai::belief::BeliefConfig;
use catan_ai::bots::{Bot, GreedyEvalBot, PlacementBot, RandomBot, SearchBot, VpGreedyBot, WeightedRandomBot};
use catan_ai::harness::run_match;
use catan_ai::eval::EvalWeights;
use catan_ai::placement::PlacementWeights;
use catan_core::game::GameConfig;

mod v2tools;

fn make_bot(name: &str, seed: u64) -> Option<Box<dyn Bot>> {
    Some(match name.to_ascii_lowercase().as_str() {
        "random" | "r" => Box::new(RandomBot::new(seed)) as Box<dyn Bot>,
        "weighted" | "w" => Box::new(WeightedRandomBot::new(seed)),
        "vpgreedy" | "vp" => Box::new(VpGreedyBot::new(seed)),
        "placement" | "p" => Box::new(PlacementBot::new(seed)),
        "greedy" | "g" => Box::new(GreedyEvalBot::new(seed)),
        "narrow" => Box::new(GreedyEvalBot::narrow(seed)) as Box<dyn Bot>,
        "greedy1" => Box::new(GreedyEvalBot::v1(seed)),
        "greedyhp" => Box::new(GreedyEvalBot::with_placement(seed, PlacementWeights::handmade(), "Greedy-配置10項目")),
        // 深さ 1。GreedyEval との差は「深さ」ではなく「作りの違い」だけになるので、
        // 先読みの効きを切り分ける物差しになる
        // 自動調整した重み。配置の重みは既定のままの版と両方置いて、最後に比べる
        "best" | "baseline" => Box::new(SearchBot::with_weights(seed, 2, EvalWeights::tuned(), PlacementWeights::tuned(), "最強候補")),
        "bestp0" => Box::new(SearchBot::with_weights(seed, 2, EvalWeights::tuned(), PlacementWeights::default(), "最強候補(配置は既定)")),
        "beste" => Box::new(SearchBot::with_weights(seed, 2, EvalWeights::default(), PlacementWeights::tuned(), "配置だけ調整")),
        "bestnarrow" => Box::new(SearchBot::narrow_shapes(seed, EvalWeights::tuned(), PlacementWeights::tuned())),
        "candidate" => Box::new(SearchBot::candidate(seed, 3, 3)),
        "bestw8" => Box::new(SearchBot::tuned_worlds(seed, 8, EvalWeights::tuned(), PlacementWeights::tuned())),
        "bestd3" => Box::new(SearchBot::with_weights(seed, 3, EvalWeights::tuned(), PlacementWeights::tuned(), "tuned-depth3")),
        "bestw3" => Box::new(SearchBot::tuned_worlds(seed, 3, EvalWeights::tuned(), PlacementWeights::tuned())),
        // v2 の対照。設計書作成時の max（2手読み・推定3通り）を凍結した別名
        "v2_control" => Box::new(catan_ai::bots::v2_control(seed)),
        // v2（観測だけを受け取る推定つきエージェント）とそのアブレーション
        "v2" => Box::new(AgentV2Bot::new(seed, V2Config::default(), "v2")),
        "v2_nohazard" => Box::new(AgentV2Bot::new(seed, V2Config { use_hazard: false, ..V2Config::default() }, "v2-ハザードなし")),
        "v2_nohist" => Box::new(AgentV2Bot::new(seed, V2Config { belief: BeliefConfig::no_history(), ..V2Config::default() }, "v2-履歴なし")),
        "v2_dur" => Box::new(AgentV2Bot::new(seed, V2Config { belief: BeliefConfig::duration_only(), ..V2Config::default() }, "v2-期間だけ")),
        "v2_feat" => Box::new(AgentV2Bot::new(seed, V2Config { belief: BeliefConfig::features_only(), ..V2Config::default() }, "v2-機会あり")),
        "v2_hw5k" => Box::new(AgentV2Bot::new(seed, V2Config { hazard_weight: 5_000.0, ..V2Config::default() }, "v2-hw5k")),
        "v2_hw100k" => Box::new(AgentV2Bot::new(seed, V2Config { hazard_weight: 100_000.0, ..V2Config::default() }, "v2-hw100k")),
        "v2_ht0" => Box::new(AgentV2Bot::new(seed, V2Config { hazard_threat: 0.0, ..V2Config::default() }, "v2-ht0")),
        "v2_ht10" => Box::new(AgentV2Bot::new(seed, V2Config { hazard_threat: 10.0, ..V2Config::default() }, "v2-ht10")),
        "v2_s1" => Box::new(AgentV2Bot::new(seed, V2Config { samples: 1, ..V2Config::default() }, "v2-s1")),
        "v2_s6" => Box::new(AgentV2Bot::new(seed, V2Config { samples: 6, ..V2Config::default() }, "v2-s6")),
        // M4: 相手の手番のロールアウトで葉を評価（研究段階）
        "v2_roll" => Box::new(AgentV2Bot::new(seed, V2Config { rollout_mix: 0.5, rollouts: 1, ..V2Config::default() }, "v2-roll0.5")),
        "v2_roll1" => Box::new(AgentV2Bot::new(seed, V2Config { rollout_mix: 1.0, rollouts: 1, ..V2Config::default() }, "v2-roll1.0")),
        "v2_roll2" => Box::new(AgentV2Bot::new(seed, V2Config { rollout_mix: 0.5, rollouts: 2, ..V2Config::default() }, "v2-roll0.5x2")),
        // 難易度の段階（Web の対戦相手と同じ物）
        "lv0" | "easy" => catan_ai::bots::bot_for_level(0, seed),
        "lv1" | "normal" => catan_ai::bots::bot_for_level(1, seed),
        "lv2" | "hard" => catan_ai::bots::bot_for_level(2, seed),
        "lv3" | "max" => catan_ai::bots::bot_for_level(3, seed),
        // 段階の間隔を測るための候補: 調整済みの重み + 1 手読み
        "lv2b" => Box::new(GreedyEvalBot::with_weights(seed, EvalWeights::tuned(), "つよい(1手読み)")),
        "search1" => Box::new(SearchBot::new(seed, 1)),
        "search" | "s" | "search2" => Box::new(SearchBot::new(seed, 2)),
        "search2n" => Box::new(SearchBot::with_budget(seed, 2, 20_000)),
        "search2s" => Box::new(SearchBot::with_budget(seed, 2, 1_000)),
        "search2w3" => Box::new(SearchBot::with_worlds(seed, 2, 3)),
        "search2w6" => Box::new(SearchBot::with_worlds(seed, 2, 6)),
        "search2k2" => Box::new(SearchBot::with_keep(seed, 2, 2.0)),
        "search2k4" => Box::new(SearchBot::with_keep(seed, 2, 4.0)),
        "search3" => Box::new(SearchBot::new(seed, 3)),
        "search4" => Box::new(SearchBot::new(seed, 4)),
        _ => return None,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("{}", USAGE);
        std::process::exit(2);
    }

    match args[0].as_str() {
        "bench" => {
            let games: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2000);
            bench(games);
        }
        "match" => run_match_cmd(&args[1..]),
        "ablate" => ablate(&args[1..]),
        "ablate2" => ablate_strong(&args[1..]),
        "sweep" => sweep(&args[1..]),
        "eval-ablate" => eval_ablate(&args[1..]),
        "eval-sweep" => eval_sweep(&args[1..]),
        "trial" => trial(&args[1..]),
        "v2-calib" => v2tools::calib(&args[1..], &make_bot),
        "v2-fit" => v2tools::fit(&args[1..], &make_bot),
        "v2-explain" => v2tools::explain(&args[1..]),
        other => {
            eprintln!("知らないコマンド: {other}\n{USAGE}");
            std::process::exit(2);
        }
    }
}

const USAGE: &str = "\
使い方:
  catan bench [試合数]
  catan ablate [試合数]   初期配置の評価項目の寄与（下流は WeightedRandom）
  catan ablate2 [試合数]  同上（下流は完成版の GreedyEval）
  catan sweep <項目> <値,値,...> [試合数] [--seed S]   重みを掃引する
  catan eval-ablate [試合数]                         評価関数の項目の寄与を測る
  catan eval-sweep <項目> <値,...> [試合数] [--seed S]
  catan v2-calib [--games N] [--seed S] [--bots a,b,c,d] [--particles P] [--only-full] [--no-hazard]
  catan v2-fit [--games N] [--seed S] [--bots a,b,c,d]     相手モデルの使用率を実測
  catan v2-explain [--seed S] [--every K]                   v2 の判断の説明を出す
  catan match <bot> <bot> [<bot>] [--games N] [--seed S] [--no-trade]
                                  [--narrow-offers] [--turn-ms MS | --no-time-limit] [--tick-ms MS]

ボット: random | weighted | vpgreedy | placement | greedy | narrow | search1 | search2 | search3 | search4

--turn-ms       1手番の持ち時間(ミリ秒・既定 120000 = 2分)。切れると交渉だけ止まる
--no-time-limit 持ち時間なし
--tick-ms       自己対戦用の仮想時計が1行動で進む量(既定 500)
--narrow-offers 提案を1種類↔1種類だけに絞る（複数種類の束を作らない）";

fn bench(games: usize) {
    println!("エンジン速度測定: ランダム 4 人 × {games} 戦");
    let mut bots: Vec<Box<dyn Bot>> = (0..4)
        .map(|i| Box::new(RandomBot::new(i)) as Box<dyn Bot>)
        .collect();
    let r = run_match(&mut bots, games, 1, GameConfig::default());
    println!("{}", r.report());
    println!(
        "  総行動数 {} / {:.0} 行動毎秒",
        r.total_actions,
        r.total_actions as f64 / r.elapsed_secs
    );
    println!(
        "\n参考: 人間の 4 人戦は平均 71 手番。ランダム同士は {:.0} 手番なので、\n\
         実戦相当の強さなら体感の {:.1} 倍速く回る。",
        r.avg_turns(),
        r.avg_turns() / 71.0
    );
}

fn run_match_cmd(args: &[String]) {
    let mut names = Vec::new();
    let mut games = 10_000usize;
    let mut seed = 1u64;
    let mut cfg = GameConfig::default();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--games" => {
                i += 1;
                games = args[i].parse().expect("--games は数値");
            }
            "--seed" => {
                i += 1;
                seed = args[i].parse().expect("--seed は数値");
            }
            "--narrow-offers" => {
                // 提案を「1 種類 ↔ 1 種類」だけに戻す（広い生成との A/B 用）
                cfg.wide_offers = false;
            }
            "--no-trade" => {
                cfg.domestic_trade = false;
                cfg.max_generated_offers = 0;
            }
            "--turn-ms" => {
                i += 1;
                cfg.turn_time_limit_ms = Some(args[i].parse().expect("--turn-ms は数値"));
            }
            "--no-time-limit" => cfg.turn_time_limit_ms = None,
            "--tick-ms" => {
                i += 1;
                cfg.turn_clock = catan_core::game::TurnClock::PerAction {
                    ms: args[i].parse().expect("--tick-ms は数値"),
                };
            }
            other => names.push(other.to_string()),
        }
        i += 1;
    }

    if !(3..=4).contains(&names.len()) {
        eprintln!("ボットは 3〜4 体指定してください\n{USAGE}");
        std::process::exit(2);
    }

    let mut bots: Vec<Box<dyn Bot>> = Vec::new();
    for (k, n) in names.iter().enumerate() {
        match make_bot(n, seed.wrapping_add(k as u64 * 7919)) {
            Some(b) => bots.push(b),
            None => {
                eprintln!("知らないボット: {n}\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    println!(
        "対戦: {} / 交易 {} / 持ち時間 {}",
        names.join(" vs "),
        if cfg.domestic_trade { "あり" } else { "なし" },
        match cfg.turn_time_limit_ms {
            None => "無制限".to_string(),
            Some(ms) => format!("{:.0}秒", ms as f64 / 1000.0),
        }
    );
    let r = run_match(&mut bots, games, seed, cfg);
    print!("{}", r.report());
}


/// 初期配置の評価関数の ablation。
///
/// 全部入り / 各項目を 1 つずつ 0 にしたもの を、それぞれ WeightedRandom×3 に当てる。
/// 「全部入りとの差」がその項目の寄与。手で決めた重みを鵜呑みにしないための手続き。
fn ablate(args: &[String]) {
    let games: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(4000);
    let cfg = GameConfig::default();
    println!("初期配置の ablation: 各条件 {games} 戦 / 相手は WeightedRandom×3");
    println!("（帰無仮説の勝率 = 25.0%。配置以外は相手と同一なので、差はまるごと配置の効果）
");

    let run = |w: PlacementWeights, label: &str| -> (f64, f64, f64) {
        let mut bots: Vec<Box<dyn Bot>> = vec![
            Box::new(PlacementBot::with_weights(1, w, label)),
            Box::new(WeightedRandomBot::new(2)),
            Box::new(WeightedRandomBot::new(3)),
            Box::new(WeightedRandomBot::new(4)),
        ];
        let r = run_match(&mut bots, games, 777, cfg);
        (r.win_rate(0), r.win_rate_stderr(0), r.avg_vp(0))
    };

    let full = PlacementWeights::default();
    let (base, base_se, base_vp) = run(full, "full");
    println!(
        "  {:<14} 勝率 {:>5.1}% ± {:.1}   平均VP {:.2}",
        "全部入り",
        base * 100.0,
        base_se * 100.0,
        base_vp
    );
    println!("  {:-<58}", "");

    let mut rows: Vec<(String, f64, f64, f64)> = Vec::new();
    for term in PlacementWeights::ALL_TERMS {
        let (p, se, vp) = run(full.without(term), term);
        rows.push((term.to_string(), p, se, vp));
    }
    // 寄与（= 全部入り − その項目を切った時）が大きい順
    rows.sort_by(|a, b| (base - b.1).partial_cmp(&(base - a.1)).unwrap());
    for (term, p, se, vp) in rows {
        let delta = (base - p) * 100.0;
        // 2 つの独立な二項比率の差の標準誤差
        let z = delta / 100.0 / (base_se * base_se + se * se).sqrt();
        println!(
            "  −{:<13} 勝率 {:>5.1}% ± {:.1}   寄与 {:+5.1}pt  平均VP {:.2}{}",
            term,
            p * 100.0,
            se * 100.0,
            delta,
            vp,
            if z.abs() > 2.58 { "  *" } else { "" }
        );
    }
    println!("
  * = 全部入りとの差が p<0.01 で有意。無印の項目は「入れても測れる効果が無い」");
}


/// 重みを 1 項目だけ掃引する。
///
/// ⚠ 多数の条件から最大値を拾うと、必ずノイズに当たりに行く。
/// 選んだ値は必ず**別の seed 集合**で確かめること（`--seed` を変えて再走）。
fn sweep(args: &[String]) {
    let term = args.first().expect("項目名が要る").clone();
    let values: Vec<f32> = args
        .get(1)
        .expect("値のリストが要る")
        .split(',')
        .map(|v| v.parse().expect("数値"))
        .collect();
    let mut games = 10000usize;
    let mut seed = 777u64;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => {
                i += 1;
                seed = args[i].parse().expect("--seed は数値");
            }
            v => games = v.parse().unwrap_or(games),
        }
        i += 1;
    }

    println!("掃引: {term} / 各条件 {games} 戦 / 相手は WeightedRandom×3 / seed {seed}");
    for v in values {
        let mut w = PlacementWeights::default();
        match term.as_str() {
            "pip" => w.pip = v,
            "variety_new" => w.variety_new = v,
            "number_dup" => w.number_dup = v,
            "same_hex" => w.same_hex = v,
            "etb_expand" => w.etb_expand = v,
            "etb_city" => w.etb_city = v,
            "etb_dev" => w.etb_dev = v,
            "port_specific" => w.port_specific = v,
            "port_generic" => w.port_generic = v,
            "expansion" => w.expansion = v,
            other => panic!("知らない項目: {other}"),
        }
        let mut bots: Vec<Box<dyn Bot>> = vec![
            Box::new(PlacementBot::with_weights(1, w, "P")),
            Box::new(WeightedRandomBot::new(2)),
            Box::new(WeightedRandomBot::new(3)),
            Box::new(WeightedRandomBot::new(4)),
        ];
        let r = run_match(&mut bots, games, seed, GameConfig::default());
        println!(
            "  {term} = {:>6.2}   勝率 {:>5.1}% ± {:.1}   平均VP {:.2}",
            v,
            r.win_rate(0) * 100.0,
            r.win_rate_stderr(0) * 100.0,
            r.avg_vp(0)
        );
    }
}


/// 評価関数の 1 条件を GreedyEval×3 に当てて勝率を返す。
/// 相手も同じボットなので、差はまるごとその項目の効果になる。
fn eval_trial(w: EvalWeights, label: &str, games: usize, seed: u64) -> (f64, f64, f64, f64) {
    let mut bots: Vec<Box<dyn Bot>> = vec![
        Box::new(GreedyEvalBot::with_weights(1, w, label)),
        Box::new(GreedyEvalBot::new(2)),
        Box::new(GreedyEvalBot::new(3)),
        Box::new(GreedyEvalBot::new(4)),
    ];
    let r = run_match(&mut bots, games, seed, GameConfig::default());
    (
        r.win_rate(0),
        r.win_rate_stderr(0),
        r.avg_vp(0),
        r.stalled as f64 / r.games as f64,
    )
}

fn eval_ablate(args: &[String]) {
    let games: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(2400);
    println!("評価関数の ablation: 各条件 {games} 戦 / 相手は GreedyEval×3（同じボット）");
    println!("（帰無仮説 25.0%。1 項目だけ 0 にして、どれだけ落ちるかを見る）\n");

    let full = EvalWeights::default();
    let (base, base_se, base_vp, base_stall) = eval_trial(full, "full", games, 4242);
    println!(
        "  {:<18} 勝率 {:>5.1}% ± {:.1}   平均VP {:.2}   膠着 {:.0}%",
        "全部入り", base * 100.0, base_se * 100.0, base_vp, base_stall * 100.0
    );
    println!("  {:-<62}", "");

    let mut rows: Vec<(String, f64, f64, f64, f64)> = Vec::new();
    for term in EvalWeights::ALL_TERMS {
        if term == "terminal" {
            continue; // 勝敗判定そのものなので外せない
        }
        let (p, se, vp, st) = eval_trial(full.without(term), term, games, 4242);
        rows.push((term.to_string(), p, se, vp, st));
    }
    rows.sort_by(|a, b| (base - b.1).partial_cmp(&(base - a.1)).unwrap());
    for (term, p, se, vp, st) in rows {
        let delta = (base - p) * 100.0;
        let z = delta / 100.0 / (base_se * base_se + se * se).sqrt();
        println!(
            "  −{:<17} 勝率 {:>5.1}% ± {:.1}   寄与 {:+5.1}pt  平均VP {:.2}  膠着 {:.0}%{}",
            term, p * 100.0, se * 100.0, delta, vp, st * 100.0,
            if z.abs() > 2.58 { "  *" } else { "" }
        );
    }
    println!("\n  * = p<0.01 で有意");
}

/// 重みを 1 組だけ差し替えたボットを、**据え置きの相手 3 体**に当てて勝率を返す。
///
/// 重みの自動調整はこれを何百回も回す。1 回 1 プロセスにして外から並列に叩けるよう、
/// 出力の最後に機械で読める 1 行を出す。
/// 相手を据え置くのは、リーグ戦にすると「自分が変わると物差しも変わる」ため。
/// 同じ seed を使い回すのは共通乱数法（同じ盤・同じ出目で比べれば差だけが残る）。
fn trial(args: &[String]) {
    // 出発点も相手も **今の最強** にする。物差しが弱いと「勝てて当たり前」になる
    let mut ew = EvalWeights::tuned();
    let mut pw = PlacementWeights::tuned();
    let mut games = 800usize;
    let mut seed = 4242u64;
    let mut depth = 2u32;
    let mut cfg = GameConfig::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--set" => {
                i += 1;
                for kv in args[i].split(',').filter(|s| !s.is_empty()) {
                    let (k, v) = kv.split_once('=').expect("--set は name=value");
                    let v: f32 = v.parse().expect("値は数値");
                    if EvalWeights::ALL_TERMS.contains(&k) {
                        ew.set(k, v);
                    } else {
                        pw.set(k, v);
                    }
                }
            }
            "--games" => { i += 1; games = args[i].parse().expect("数値"); }
            "--seed" => { i += 1; seed = args[i].parse().expect("数値"); }
            "--no-time-limit" => cfg.turn_time_limit_ms = None,
            "--depth" => { i += 1; depth = args[i].parse().expect("数値"); }
            other => panic!("知らない引数: {other}"),
        }
        i += 1;
    }
    let mut bots: Vec<Box<dyn Bot>> = vec![
        Box::new(SearchBot::with_weights(1, depth, ew, pw, "候補")),
        Box::new(SearchBot::with_weights(2, depth, EvalWeights::tuned(), PlacementWeights::tuned(), "現行")),
        Box::new(SearchBot::with_weights(3, depth, EvalWeights::tuned(), PlacementWeights::tuned(), "現行")),
        Box::new(SearchBot::with_weights(4, depth, EvalWeights::tuned(), PlacementWeights::tuned(), "現行")),
    ];
    let r = run_match(&mut bots, games, seed, cfg);
    println!(
        "RESULT win={:.5} se={:.5} vp={:.3} stall={:.4} games={}",
        r.win_rate(0),
        r.win_rate_stderr(0),
        r.avg_vp(0),
        r.stalled as f64 / r.games as f64,
        r.games
    );
}

fn eval_sweep(args: &[String]) {
    let term = args.first().expect("項目名が要る").clone();
    let values: Vec<f32> = args
        .get(1)
        .expect("値のリストが要る")
        .split(',')
        .map(|v| v.parse().expect("数値"))
        .collect();
    let mut games = 2400usize;
    let mut seed = 4242u64;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => {
                i += 1;
                seed = args[i].parse().expect("--seed は数値");
            }
            v => games = v.parse().unwrap_or(games),
        }
        i += 1;
    }
    println!("評価関数の掃引: {term} / 各条件 {games} 戦 / 相手は GreedyEval×3 / seed {seed}");
    for v in values {
        let mut w = EvalWeights::default();
        w.set(&term, v);
        let (p, se, vp, st) = eval_trial(w, "P", games, seed);
        println!(
            "  {term} = {:>9.2}   勝率 {:>5.1}% ± {:.1}   平均VP {:.2}   膠着 {:.0}%",
            v, p * 100.0, se * 100.0, vp, st * 100.0
        );
    }
}


/// 初期配置の ablation を、**強い下流の下で**やり直す。
///
/// 最初の測定は「配置以外が WeightedRandom」だった。産出の散らし・港・ETB は
/// 計画的に打てる打ち手を持って初めて価値が出る種類の特徴量なので、
/// 下流が強くなったら測り直す、と約束していたぶん。
fn ablate_strong(args: &[String]) {
    let games: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(2400);
    let cfg = GameConfig::default();
    println!("初期配置の ablation（下流 = 完成版 GreedyEval）: 各条件 {games} 戦");
    println!("相手も同じボット。差はまるごと配置の重みの効果
");

    let run = |w: PlacementWeights, label: &str| -> (f64, f64, f64) {
        let mut bots: Vec<Box<dyn Bot>> = vec![
            Box::new(GreedyEvalBot::with_placement(1, w, label)),
            Box::new(GreedyEvalBot::new(2)),
            Box::new(GreedyEvalBot::new(3)),
            Box::new(GreedyEvalBot::new(4)),
        ];
        let r = run_match(&mut bots, games, 31337, cfg);
        (r.win_rate(0), r.win_rate_stderr(0), r.avg_vp(0))
    };

    let full = PlacementWeights::default();
    let (base, base_se, base_vp) = run(full, "full");
    println!(
        "  {:<16} 勝率 {:>5.1}% ± {:.1}   平均VP {:.2}",
        "確定版(2項目)", base * 100.0, base_se * 100.0, base_vp
    );
    let (h, h_se, h_vp) = run(PlacementWeights::handmade(), "handmade");
    println!(
        "  {:<16} 勝率 {:>5.1}% ± {:.1}   平均VP {:.2}",
        "手で決めた版", h * 100.0, h_se * 100.0, h_vp
    );
    println!("  {:-<58}", "");

    // 0 にしてある項目を足し戻して、いま価値が出るかを見る
    for (term, v) in [
        ("number_dup", -0.5f32),
        ("same_hex", -3.0),
        ("port_specific", 3.0),
        ("port_generic", 1.5),
        ("expansion", 0.6),
        ("etb_expand", 8.0),
        ("etb_city", 6.0),
    ] {
        let mut w = full;
        match term {
            "number_dup" => w.number_dup = v,
            "same_hex" => w.same_hex = v,
            "port_specific" => w.port_specific = v,
            "port_generic" => w.port_generic = v,
            "expansion" => w.expansion = v,
            "etb_expand" => w.etb_expand = v,
            "etb_city" => w.etb_city = v,
            _ => {}
        }
        let (p, se, vp) = run(w, term);
        let d = (p - base) * 100.0;
        let z = d / 100.0 / (base_se * base_se + se * se).sqrt();
        println!(
            "  +{:<15} 勝率 {:>5.1}% ± {:.1}   差 {:+5.1}pt  平均VP {:.2}{}",
            term, p * 100.0, se * 100.0, d, vp,
            if z.abs() > 2.58 { "  *" } else { "" }
        );
    }
    println!("
  * = p<0.01 で有意");
}
