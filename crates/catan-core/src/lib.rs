//! カタン（基本セット・3〜4人）のルールエンジン。
//!
//! 方針:
//! - 外部依存ゼロ。決定的（同じ seed で同じ試合になる）。
//! - `Game` は複製が安いこと。探索（expectimax）が状態を丸ごとコピーするため。
//! - 「意図」と「結果」を分ける。`Action` は意図だけを持ち、ダイス目や盗んだカードは
//!   `ActionRecord.result` に入る。これでリプレイ・巻き戻し・学習データ生成が同じ型で回る。

pub mod action;
pub mod board;
pub mod coord;
pub mod game;
pub mod longest_road;
pub mod rng;
pub mod topology;
pub mod view;

pub use board::{Board, BoardConfig, BuildingKind, NumberPlacement, PlayerId, PortKind, Resource, RESOURCES};
pub use coord::{Coord, Direction};
pub use topology::{EdgeId, NodeId, TileId, Topology, NUM_EDGES, NUM_NODES, NUM_TILES};
pub use action::{Action, ActionRecord, Bundle, DevCard, Outcome, Prompt};
pub use game::{Game, GameConfig, PlayerState};
pub use view::View;
