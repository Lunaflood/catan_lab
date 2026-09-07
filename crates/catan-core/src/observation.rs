//! 観測契約（席ごとに「その人が知り得るもの」だけを運ぶ型）。
//!
//! 何を公開するかの線引きは `docs/cpu-v2/observation-contract.md` の表で決めている。
//! ここにある型は **本番 `Game` への参照を持たない**。値として切り出した公開情報だけを持つ。
//!
//! - [`Observation`]: 判断を求められた瞬間のスナップショット（公開状態＋本人だけの私的状態＋合法手）
//! - [`ObservedEvent`]: 1 つの行動が起きたことを、その席から見える範囲で伝える
//!
//! `Observation.public.game` は「秘密を消した局面」。相手の資源・発展カード・
//! 今手番に買った札の内訳・山の内訳・本番の乱数状態を 0 または定数に置き換えてある。
//! 同じ観測履歴を生む本番状態どうしでは、この値は bit 単位で一致する（テスト `fairness_v2`）。

use crate::action::{Action, Bundle, DevCard, Prompt, NUM_DEV_KINDS};
use crate::board::{PlayerId, Resource};
use crate::game::{Game, MAX_PLAYERS};
use crate::topology::{EdgeId, NodeId, TileId};

/// ルールの版。情報公開の版（[`ObservationProfile`]）とは別に管理する。
pub const RULES_VERSION: &str = "base-5th-2020/v1";

/// 情報公開の契約。プロファイルが違う対局は同じ勝率表に混ぜない。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ObservationProfile {
    pub name: &'static str,
    /// 銀行の資源別残数を正確に公開するか（false なら 0 で埋める）
    pub bank_exact: bool,
    /// 7 の廃棄の内訳を第三者に公開するか
    pub discard_public: bool,
    /// 盗みの資源種類を当事者以外にも公開するか（false = 奪った人と奪われた人だけ）
    pub steal_kind_public: bool,
    /// 対案の候補の積み下ろし（`CounterAlt` / `CounterAltRemove`）を第三者に見せるか
    pub counter_drafts_public: bool,
}

/// 既存 UI（Web / オンライン）が実際に人間へ見せている範囲。移行中の固定値。
pub const LEGACY_APP: ObservationProfile = ObservationProfile {
    name: "legacy_app",
    bank_exact: true,
    discard_public: true,
    steal_kind_public: false,
    counter_drafts_public: true,
};

/// 1 人分の公開情報（枚数だけ）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PublicPlayer {
    pub hand_size: u8,
    pub dev_count: u8,
    /// 今手番に買った発展カードの枚数（種類は非公開）
    pub dev_bought_this_turn: u8,
    /// これまでに使った発展カード（種類別）。公開
    pub played_dev: [u8; NUM_DEV_KINDS],
    pub public_vp: u8,
}

/// 公開状態
#[derive(Clone, PartialEq, Debug)]
pub struct PublicState {
    /// 秘密を消した局面。相手の手札・発展・山の内訳・本番 RNG は入っていない。
    /// 自分の手札も入っていない（[`OwnState`] に分けてある）。
    pub game: Game,
    pub players: [PublicPlayer; MAX_PLAYERS],
    /// 山に残っている発展カードの枚数（種類は非公開）
    pub dev_deck_left: u8,
    /// 銀行の残数。プロファイルが非公開なら `None`
    pub bank: Option<Bundle>,
}

/// 本人だけが知る状態
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct OwnState {
    pub hand: Bundle,
    pub dev: [u8; NUM_DEV_KINDS],
    pub dev_bought_this_turn: [u8; NUM_DEV_KINDS],
    /// 伏せた勝利点カードを含む自分の実際の勝利点
    pub actual_vp: u8,
}

/// 判断を求められた瞬間の観測
#[derive(Clone, PartialEq, Debug)]
pub struct Observation {
    pub viewer: PlayerId,
    pub rules_version: &'static str,
    pub profile: ObservationProfile,
    /// これまでに届いた出来事の数（ホストが数える。分からなければ 0）
    pub seq: u64,
    pub public: PublicState,
    pub own: OwnState,
    /// その席に実際に提示できる合法手だけ
    pub legal: Vec<Action>,
}

impl Observation {
    #[inline]
    pub fn num_players(&self) -> usize {
        self.public.game.n()
    }
}

/// その席から見える出来事の中身
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VisibleEvent {
    SetupSettlement { node: NodeId, gained: Bundle },
    SetupRoad { edge: EdgeId },
    /// 出目と、各人が受け取った資源（産出は全員に見える）
    Roll { d1: u8, d2: u8, produced: [Bundle; MAX_PLAYERS] },
    /// 廃棄。内訳はプロファイルと当事者性で伏せられる
    Discard { count: u8, bundle: Option<Bundle> },
    /// 盗賊の移動。`stolen` は当事者（または公開プロファイル）だけ `Some`。
    /// `stole_any` は「1 枚移ったか」で、これは全員に見える
    RobberMoved { tile: TileId, victim: Option<PlayerId>, stole_any: bool, stolen: Option<Resource> },
    BuildRoad { edge: EdgeId, free: bool },
    BuildSettlement { node: NodeId },
    BuildCity { node: NodeId },
    /// 発展カードの購入。種類は本人だけ `Some`
    BuyDev { card: Option<DevCard> },
    PlayKnight,
    PlayRoadBuilding,
    PlayYearOfPlenty { a: Resource, b: Resource },
    /// 独占。各人が渡した枚数は全員に見える
    PlayMonopoly { res: Resource, taken: [u8; MAX_PLAYERS] },
    Maritime { give: Resource, count: u8, take: Resource },
    Offer { give: Bundle, want: Bundle },
    Accept,
    Reject,
    CounterAlt { give: Bundle, want: Bundle },
    CounterAltRemove { index: u8 },
    Counter { give: Bundle, want: Bundle },
    /// 提案者が `give` を出し、`partner` が `want` を出して成立
    Confirm { partner: PlayerId, give: Bundle, want: Bundle },
    /// 対案で成立。`partner` が `partner_gives` を出し、提案者が `proposer_gives` を出す
    AcceptCounter { partner: PlayerId, alt: u8, partner_gives: Bundle, proposer_gives: Bundle },
    Cancel,
    EndTurn,
    /// プロファイルで隠された出来事。順序番号だけ進む
    Hidden,
}

/// 出来事が起きた**直前**の公開状態の要点。相手モデル（使用機会の特徴量）に使う。
/// すべて公開情報から計算している。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct EventContext {
    pub robber: TileId,
    /// 盗賊がそのプレイヤーの産出を何 pip 止めているか
    pub blocked_pips: [u16; MAX_PLAYERS],
    pub played_knights: [u8; MAX_PLAYERS],
    pub largest_army_owner: Option<PlayerId>,
    pub longest_road: [u8; MAX_PLAYERS],
    pub longest_road_owner: Option<PlayerId>,
    pub roads_left: [u8; MAX_PLAYERS],
    /// 合法な道の置き先があるか
    pub can_build_road: [bool; MAX_PLAYERS],
    pub settlements_left: [u8; MAX_PLAYERS],
    pub cities_left: [u8; MAX_PLAYERS],
    /// 自分の道網から今すぐ建てられる頂点の数
    pub buildable_nodes: [u8; MAX_PLAYERS],
    pub bank: Option<Bundle>,
    pub hand_size: [u8; MAX_PLAYERS],
    pub dev_count: [u8; MAX_PLAYERS],
    pub public_vp: [u8; MAX_PLAYERS],
    /// 手番プレイヤーがこの手番に発展カードを使ったか
    pub dev_played_this_turn: bool,
    pub rolled: bool,
}

/// 出来事の**直後**の公開の要点（推定の自己検査に使う）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EventAftermath {
    pub hand_size: [u8; MAX_PLAYERS],
    pub dev_count: [u8; MAX_PLAYERS],
    pub dev_bought_this_turn: [u8; MAX_PLAYERS],
    pub public_vp: [u8; MAX_PLAYERS],
    pub dev_deck_left: u8,
    pub bank: Option<Bundle>,
    pub longest_road_owner: Option<PlayerId>,
    pub largest_army_owner: Option<PlayerId>,
    pub turn_player: PlayerId,
    pub to_act: PlayerId,
    pub prompt: Prompt,
    pub winner: Option<PlayerId>,
}

impl Default for EventAftermath {
    fn default() -> Self {
        EventAftermath {
            hand_size: [0; MAX_PLAYERS],
            dev_count: [0; MAX_PLAYERS],
            dev_bought_this_turn: [0; MAX_PLAYERS],
            public_vp: [0; MAX_PLAYERS],
            dev_deck_left: 0,
            bank: None,
            longest_road_owner: None,
            largest_army_owner: None,
            turn_player: 0,
            to_act: 0,
            prompt: Prompt::PlayTurn,
            winner: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ObservedEvent {
    pub seq: u64,
    pub viewer: PlayerId,
    pub actor: PlayerId,
    /// 行動が起きた時の場面
    pub prompt: Prompt,
    /// 行動が起きた時の手番番号（`Game::turn`）と手番プレイヤー
    pub turn: u32,
    pub turn_player: PlayerId,
    pub num_players: u8,
    pub payload: VisibleEvent,
    pub before: EventContext,
    pub after: EventAftermath,
}
