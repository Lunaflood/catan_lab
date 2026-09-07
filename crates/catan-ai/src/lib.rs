//! ボットと評価ハーネス。
//!
//! 評価の作法は先行研究の失敗から決めている（docs/catan-research.md 参照）:
//! - **1 条件 10,000 戦**。26 戦程度では実力差が統計的に出ない。
//! - **席順を必ず入れ替える**。席バイアスは有意で、しかも戦略の強さで符号が反転する
//!   （ランダム同士では 1 番手有利、MCTS 同士では 2・3 番手有利: Szita et al. 2010）。
//! - 勝率だけでなく **ヘクスからの資源獲得・建てた駒の種類比率** を見る。
//!   どこが効いているかはここに出る（Guhe & Lascarides 2014）。

pub mod agent_v2;
pub mod belief;
pub mod bots;
pub mod eval;
pub mod harness;
pub mod history;
pub mod opponent;
pub mod placement;
pub mod prune;
pub mod search;
pub mod search_v2;
pub mod trade;

pub use bots::{Bot, GreedyEvalBot, PlacementBot, RandomBot, SearchBot, VpGreedyBot, WeightedRandomBot};
pub use eval::EvalWeights;
pub use placement::PlacementWeights;
pub use harness::{run_match, MatchResult};
