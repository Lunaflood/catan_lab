//! ゲーム状態とルール適用。
//!
//! `Game` はヒープ確保を一切持たない（`dev_deck` も残枚数の配列）。
//! これは探索が状態を丸ごと複製するため。`Clone` は memcpy 相当。

use crate::action::*;
use crate::board::*;
use crate::longest_road::{longest_road_length, resolve_longest_road};
use crate::rng::Rng;
use crate::topology::{EdgeId, NodeId, TileId, Topology, NUM_EDGES, NUM_NODES, NUM_TILES};

pub const MAX_PLAYERS: usize = 4;
pub const VP_TO_WIN: u8 = 10;
/// 銀行の各資源の枚数（公式: 資源カード 95 枚 = 各 19 枚）
pub const BANK_PER_RESOURCE: u8 = 19;
/// これを**超える**枚数（= 8 枚以上）を持っていると 7 で廃棄
pub const HAND_LIMIT: u8 = 7;
pub const MAX_ROADS: u8 = 15;
pub const MAX_SETTLEMENTS: u8 = 5;
pub const MAX_CITIES: u8 = 4;
/// 最大騎士力に必要な騎士カード枚数
pub const MIN_ARMY: u8 = 3;
/// 1 人が返せる対案の候補数。「木か土ならいい」を表すのに要る。
///
/// 卓上の交渉には「どれかで手を打つ」という言い方があるが、条件 1 本では書けない。
/// 候補を並べて **提案者に選ばせる** ことで表す。
pub const MAX_COUNTER_ALTS: usize = 4;

pub const COST_ROAD: Bundle = [1, 1, 0, 0, 0];
pub const COST_SETTLEMENT: Bundle = [1, 1, 1, 1, 0];
pub const COST_CITY: Bundle = [0, 0, 0, 2, 3];
pub const COST_DEV: Bundle = [0, 0, 1, 1, 1];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlayerState {
    pub hand: Bundle,
    /// 手札の発展カード（種類別）
    pub dev: [u8; NUM_DEV_KINDS],
    /// このターンに買った分。ルール上このターンは使えない
    pub dev_bought_this_turn: [u8; NUM_DEV_KINDS],
    /// これまでに打った発展カード（種類別）。**全員に見える公開情報**。
    /// 騎士は場に表向きで残り、進歩カードは使うとゲームから除外されるので、
    /// どちらも「誰が何を何枚使ったか」は追える。カードカウンティングの土台。
    pub played_dev: [u8; NUM_DEV_KINDS],
    pub roads_left: u8,
    pub settlements_left: u8,
    pub cities_left: u8,
    /// 最長路の長さ（建設のたびに更新するキャッシュ）
    pub longest_road: u8,
}

impl Default for PlayerState {
    fn default() -> Self {
        PlayerState {
            hand: EMPTY,
            dev: [0; NUM_DEV_KINDS],
            dev_bought_this_turn: [0; NUM_DEV_KINDS],
            played_dev: [0; NUM_DEV_KINDS],
            roads_left: MAX_ROADS,
            settlements_left: MAX_SETTLEMENTS,
            cities_left: MAX_CITIES,
            longest_road: 0,
        }
    }
}

impl PlayerState {
    #[inline]
    pub fn hand_size(&self) -> u8 {
        bundle_total(&self.hand)
    }
    /// このターンに使える発展カードの枚数
    #[inline]
    pub fn playable_dev(&self, c: DevCard) -> u8 {
        self.dev[c.idx()] - self.dev_bought_this_turn[c.idx()]
    }
    #[inline]
    pub fn dev_count(&self) -> u8 {
        self.dev.iter().sum()
    }
    /// 場に出ている騎士の枚数（最大騎士力の判定に使う）
    #[inline]
    pub fn played_knights(&self) -> u8 {
        self.played_dev[DevCard::Knight.idx()]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TradeState {
    pub proposer: PlayerId,
    pub give: Bundle,
    pub want: Bundle,
    /// 次に諾否を答えるプレイヤー
    pub responder: PlayerId,
    pub accepted: [bool; MAX_PLAYERS],
    /// 各プレイヤーが返した対案の候補。`(その人が出す, その人が欲しい)`。
    /// 「木か土ならいい」のように複数並べられる（[`MAX_COUNTER_ALTS`] 本まで）。
    pub counters: [[Option<(Bundle, Bundle)>; MAX_COUNTER_ALTS]; MAX_PLAYERS],
    /// もう諾否を答えた席。順番を席順から外すので、人数の数だけでは足りない
    pub responded: [bool; MAX_PLAYERS],
    pub answered: u8,
}

/// 手番の持ち時間をどう進めるか。
///
/// エンジンは「同じ seed なら同じ試合」を保証したいので、実時間を直接読まない。
/// 時計は必ず外から与える。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TurnClock {
    /// 実時間。ホスト（UI）が [`Game::tick`] で進める。人間との対戦用。
    Realtime,
    /// 1 行動ごとに固定量だけ進む仮想時計。決定的なので自己対戦・探索用。
    PerAction { ms: u32 },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GameConfig {
    pub board: BoardConfig,
    /// プレイヤー間交易を有効にするか。
    /// ⚠ 最強OSSの catanatron はこれを行動空間に持っていない（= 交渉なしカタン）。
    ///   本エンジンでは既定で有効。
    pub domestic_trade: bool,
    /// `legal_actions` が自動生成する提案の上限。0 なら提案を生成しない
    /// （AI 側が自前で `OfferTrade` を作る想定。`apply` は任意の合法提案を受け付ける）。
    pub max_generated_offers: usize,
    /// `legal_actions` が返す廃棄候補の上限。超える場合は代表例に間引く。
    /// `apply` は上限に関係なく任意の合法な廃棄を受け付ける。
    pub max_discard_actions: usize,
    /// 提案の諾否を**最後に回す席**（ビットで指定。bit0 = 席0）。
    ///
    /// ここに入れた席は、他の全員が答え終わってから聞かれる。
    /// 人間の席を入れる想定 ── CPU は即答するので、人間が答える時点で
    /// **他全員の返事が出そろっている**（＝一斉に聞かれたのと同じ見え方になる）。
    /// 0 なら従来どおり席順。⚠ 自己対戦（全員 CPU）は 0 のままなので、
    /// 既存の対戦成績・テストの再現性は変わらない。
    pub answer_last_mask: u8,
    /// 自動生成する提案に **複数種類の束** を含めるか。
    ///
    /// 公式ルールに枚数の制限は無い（禁じられているのは同種を両側に置くことだけ）。
    /// 既定の狭い生成では「1 種類 ↔ 1 種類」しか出ないので、
    /// 「木 1 と土 1 を出して鉄 1 が欲しい」のような、実戦で最も要る形が作れない。
    /// `apply` は最初から任意の合法提案を受けるので、これは **AI の探索範囲** の話。
    pub wide_offers: bool,
    /// 手番の持ち時間（ミリ秒）。`None` で無制限。
    ///
    /// 公式ルールに持ち時間の規定は無いが、交渉には終わりが要る
    /// （無いと「提案 → 全員拒否 → また提案」で手番が終わらない）。
    /// **時間切れで止まるのは交渉だけ**で、建設や手番終了はそのまま続けられる。
    /// 提案を打ち切れば手番は必ず有限で終わる（海上交易も建設も資源を減らすため）。
    pub turn_time_limit_ms: Option<u32>,
    /// 持ち時間の進め方。UI から使う時は [`TurnClock::Realtime`] にして
    /// [`Game::tick`] で実時間を流し込む。
    pub turn_clock: TurnClock,
}

impl Default for GameConfig {
    fn default() -> Self {
        GameConfig {
            board: BoardConfig::default(),
            domestic_trade: true,
            max_generated_offers: 200,
            wide_offers: true,
            max_discard_actions: 64,
            answer_last_mask: 0,
            turn_time_limit_ms: Some(2 * 60 * 1000),
            // 既定は決定的な仮想時計。2 分 ÷ 500ms = 1 手番あたり 240 行動ぶんの
            // 交渉予算になり、通常の対局では絶対に当たらないが停止性は保証される。
            turn_clock: TurnClock::PerAction { ms: 500 },
        }
    }
}

/// 探索用: 偶然の結果を自前で指定したい時に使う
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Forced {
    No,
    Dice(u8, u8),
    Draw(DevCard),
    Steal(Option<Resource>),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Game {
    pub cfg: GameConfig,
    pub board: Board,
    pub players: [PlayerState; MAX_PLAYERS],
    pub num_players: u8,
    pub bank: Bundle,
    /// 発展カードの残り枚数（山をシャッフルして持つ代わりに、引く時に重み付き抽選する。
    /// 統計的に等価で、複製が無料になり、expectimax で分岐を列挙しやすい）
    pub dev_deck: [u8; NUM_DEV_KINDS],
    /// 出来事ごとに **別々の流れ** を持つ。
    ///
    /// 1 本の流れを共有すると、発展カードを 1 枚多く引いただけで
    /// 以降のサイコロが丸ごとずれる（＝出来事どうしが絡む）。
    /// 質としての「ランダムさ」は変わらないが、**独立に並ぶ**ようになる。
    pub rng_dice: Rng,
    pub rng_dev: Rng,
    pub rng_steal: Rng,

    pub turn_player: PlayerId,
    /// いま選択を求められているプレイヤー（廃棄・交易の諾否では手番プレイヤーと異なる）
    pub to_act: PlayerId,
    pub prompt: Prompt,
    pub turn: u32,
    pub rolled: bool,
    pub dev_played_this_turn: bool,
    pub free_roads: u8,
    /// この手番で既に出した交易提案の回数（統計用。手を制限はしない）
    pub offers_this_turn: u8,
    /// この手番で**断られた**提案。同じものを出し直せないようにする。
    ///
    /// 断られた条件をそのまま出し直しても答えは変わらないので、手番が進まなくなる。
    /// （実際にブラウザで対局していて、CPU が同じ提案を延々と繰り返して止まった）
    /// 自動生成する提案の形は 120 通りしかないのでビットで持てる。
    /// AI が自前で組んだそれ以外の提案は対象外（[`canonical_offer_index`] が `None` を返す）。
    /// この手番で断られた提案の集合。形の通し番号をビットで持つ。
    /// **探索で何万回も複製されるので Vec は置けない**。180 通りを 256 ビットに収める
    pub rejected_offers: [u128; 2],
    /// この手番の経過時間。[`TurnClock`] の設定に従って進む
    pub turn_elapsed_ms: u32,

    /// 初期配置の進行度（0..2*num_players）
    pub setup_index: u8,
    /// 初期配置で直前に置いた開拓地（道はここに接する必要がある）
    pub last_setup_node: NodeId,

    pub longest_road_owner: Option<PlayerId>,
    pub largest_army_owner: Option<PlayerId>,

    /// 各プレイヤーが捨てるべき枚数（7 の処理中のみ非ゼロ）
    pub discard_pending: [u8; MAX_PLAYERS],
    pub trade: Option<TradeState>,
    pub winner: Option<PlayerId>,
}

impl Game {
    pub fn new(num_players: u8, seed: u64) -> Game {
        Game::with_config(num_players, seed, GameConfig::default())
    }

    pub fn with_config(num_players: u8, seed: u64, cfg: GameConfig) -> Game {
        Game::with_seeds(num_players, seed, seed, cfg)
    }

    /// 盤面と出目に **別々の種** を与える。出目側は内部で
    /// サイコロ・発展カード・盗みの 3 本に分ける。
    pub fn with_seeds(num_players: u8, board_seed: u64, play_seed: u64, cfg: GameConfig) -> Game {
        Game::with_streams(
            num_players,
            board_seed,
            splitmix64(play_seed ^ 0xA1),
            splitmix64(play_seed ^ 0xB2),
            splitmix64(play_seed ^ 0xC3),
            cfg,
        )
    }

    /// 出来事ごとに種を分けて始める。
    ///
    /// 盤・サイコロ・発展カード・盗みが**互いに影響しない**。
    /// 1 本の流れを共有していると、たとえば誰かが発展カードを 1 枚引いただけで
    /// 以降のサイコロが全部ずれる。分けておけば「盤はそのまま、出目だけ変える」も効く。
    pub fn with_streams(
        num_players: u8,
        board_seed: u64,
        dice_seed: u64,
        dev_seed: u64,
        steal_seed: u64,
        cfg: GameConfig,
    ) -> Game {
        assert!(
            (3..=MAX_PLAYERS as u8).contains(&num_players),
            "基本セットは 3〜4 人"
        );
        let mut board_rng = Rng::with_stream(board_seed, 0x9e37_79b9_7f4a_7c15);
        let board = Board::generate(&mut board_rng, cfg.board);
        let mut dev_deck = [0u8; NUM_DEV_KINDS];
        for (c, n) in DEV_DECK_COMPOSITION {
            dev_deck[c.idx()] = n;
        }
        Game {
            cfg,
            board,
            players: [PlayerState::default(); MAX_PLAYERS],
            num_players,
            bank: [BANK_PER_RESOURCE; NUM_RESOURCES],
            dev_deck,
            rng_dice: Rng::with_stream(dice_seed, 0xda3e_39cb_94b9_5bdb),
            rng_dev: Rng::with_stream(dev_seed, 0x2545_f491_4f6c_dd1d),
            rng_steal: Rng::with_stream(steal_seed, 0x1405_7b7e_f767_814f),
            turn_player: 0,
            to_act: 0,
            prompt: Prompt::SetupSettlement,
            turn: 0,
            rolled: false,
            dev_played_this_turn: false,
            free_roads: 0,
            offers_this_turn: 0,
            rejected_offers: [0; 2],
            turn_elapsed_ms: 0,
            setup_index: 0,
            last_setup_node: 0,
            longest_road_owner: None,
            largest_army_owner: None,
            discard_pending: [0; MAX_PLAYERS],
            trade: None,
            winner: None,
        }
    }

    #[inline]
    pub fn n(&self) -> usize {
        self.num_players as usize
    }

    #[inline]
    pub fn is_over(&self) -> bool {
        self.winner.is_some()
    }

    #[inline]
    pub fn is_setup(&self) -> bool {
        self.setup_index < 2 * self.num_players
    }

    /// 初期配置で `i` 番目に置くプレイヤー（1 巡目は順回り、2 巡目は逆回り）
    #[inline]
    fn setup_actor(&self, i: u8) -> PlayerId {
        let n = self.num_players;
        if i < n {
            i
        } else {
            2 * n - 1 - i
        }
    }

    #[inline]
    fn next_player(&self, p: PlayerId) -> PlayerId {
        (p + 1) % self.num_players
    }

    /// 次に諾否を聞く相手。
    /// まだ答えていない席のうち、`answer_last_mask` に**入っていない**席を先に回し、
    /// 入っている席（人間）は最後に残す。全員答え終わっていれば `None`。
    fn next_responder(&self, t: &TradeState) -> Option<PlayerId> {
        let n = self.num_players;
        let mut last: Option<PlayerId> = None;
        for k in 1..n {
            let p = (t.proposer + k) % n;
            if t.responded[p as usize] {
                continue;
            }
            if self.cfg.answer_last_mask & (1 << p) != 0 {
                if last.is_none() {
                    last = Some(p);
                }
                continue;
            }
            return Some(p);
        }
        last
    }

    // =====================================================================
    // 手番の持ち時間
    // =====================================================================

    /// 実時間を流し込む。[`TurnClock::Realtime`] の時に UI から呼ぶ。
    pub fn tick(&mut self, ms: u32) {
        self.turn_elapsed_ms = self.turn_elapsed_ms.saturating_add(ms);
    }

    /// 手番の残り時間。無制限なら `None`。
    pub fn turn_time_left_ms(&self) -> Option<u32> {
        self.cfg
            .turn_time_limit_ms
            .map(|lim| lim.saturating_sub(self.turn_elapsed_ms))
    }

    /// まだ交渉できるか（提案・対案を出せるか）。
    /// 時間切れでも建設と手番終了は続けられる。
    pub fn negotiation_open(&self) -> bool {
        match self.cfg.turn_time_limit_ms {
            None => true,
            Some(lim) => self.turn_elapsed_ms < lim,
        }
    }

    // =====================================================================
    // 勝利点
    // =====================================================================

    /// 公開されている勝利点（伏せた勝利点カードを含まない）
    pub fn public_vp(&self, p: PlayerId) -> u8 {
        let mut vp = 0;
        for n in 0..NUM_NODES {
            if let Some(b) = self.board.building[n] {
                if b.owner == p {
                    vp += match b.kind {
                        BuildingKind::Settlement => 1,
                        BuildingKind::City => 2,
                    };
                }
            }
        }
        if self.longest_road_owner == Some(p) {
            vp += 2;
        }
        if self.largest_army_owner == Some(p) {
            vp += 2;
        }
        vp
    }

    /// 伏せた勝利点カードを含む実際の勝利点
    pub fn actual_vp(&self, p: PlayerId) -> u8 {
        self.public_vp(p) + self.players[p as usize].dev[DevCard::VictoryPoint.idx()]
    }

    // =====================================================================
    // 合法手の生成
    // =====================================================================

    pub fn legal_actions(&self) -> Vec<Action> {
        let mut out = Vec::with_capacity(32);
        self.legal_actions_into(&mut out);
        out
    }

    pub fn legal_actions_into(&self, out: &mut Vec<Action>) {
        out.clear();
        let topo = Topology::get();
        let p = self.to_act;
        let ps = &self.players[p as usize];

        match self.prompt {
            Prompt::GameOver => {}

            Prompt::SetupSettlement => {
                for n in 0..NUM_NODES {
                    if self.can_place_settlement(n as NodeId, p, true) {
                        out.push(Action::SetupSettlement(n as NodeId));
                    }
                }
            }

            Prompt::SetupRoad => {
                for e in &topo.node_edges[self.last_setup_node as usize] {
                    if self.board.road[e as usize].is_none() {
                        out.push(Action::SetupRoad(e));
                    }
                }
            }

            Prompt::Discard => {
                self.gen_discards(p, out);
            }

            Prompt::MoveRobber => {
                self.gen_robber_moves(p, out);
            }

            Prompt::FreeRoad => {
                for e in 0..NUM_EDGES {
                    if self.can_build_road(e as EdgeId, p) {
                        out.push(Action::BuildRoad(e as EdgeId));
                    }
                }
            }

            Prompt::DecideTrade => {
                let t = self.trade.expect("交易中でないのに DecideTrade");
                out.push(Action::RejectTrade);
                // 求められている資源を持っている時だけ承諾できる
                if bundle_contains(&ps.hand, &t.want) {
                    out.push(Action::AcceptTrade);
                }
                // 対案。時間切れなら出せない
                if self.negotiation_open() && self.cfg.max_generated_offers > 0 {
                    self.gen_offers(p, out, true);
                }
            }

            Prompt::DecideAcceptees => {
                let t = self.trade.expect("交易中でないのに DecideAcceptees");
                out.push(Action::CancelTrade);
                for q in 0..self.n() {
                    if t.accepted[q] {
                        out.push(Action::ConfirmTrade(q as PlayerId));
                    }
                    // 対案は、提案者が求められた側の資源を払える時だけ受けられる。
                    // 候補が複数あれば、払えるものを全部並べて選ばせる
                    for (k, c) in t.counters[q].iter().enumerate() {
                        if let Some((_cgive, cwant)) = c {
                            if bundle_contains(&ps.hand, cwant) {
                                out.push(Action::AcceptCounter { from: q as PlayerId, alt: k as u8 });
                            }
                        }
                    }
                }
            }

            Prompt::PlayTurn => {
                // 発展カードはダイスの前後どちらでも打てる（公式アルマナック）
                self.gen_dev_plays(p, out);

                if !self.rolled {
                    out.push(Action::Roll);
                    return;
                }

                out.push(Action::EndTurn);

                // 建設
                if ps.roads_left > 0 && bundle_contains(&ps.hand, &COST_ROAD) {
                    for e in 0..NUM_EDGES {
                        if self.can_build_road(e as EdgeId, p) {
                            out.push(Action::BuildRoad(e as EdgeId));
                        }
                    }
                }
                if ps.settlements_left > 0 && bundle_contains(&ps.hand, &COST_SETTLEMENT) {
                    for n in 0..NUM_NODES {
                        if self.can_place_settlement(n as NodeId, p, false) {
                            out.push(Action::BuildSettlement(n as NodeId));
                        }
                    }
                }
                if ps.cities_left > 0 && bundle_contains(&ps.hand, &COST_CITY) {
                    for n in 0..NUM_NODES {
                        if self.board.building[n]
                            == Some(Building {
                                owner: p,
                                kind: BuildingKind::Settlement,
                            })
                        {
                            out.push(Action::BuildCity(n as NodeId));
                        }
                    }
                }
                if self.dev_deck.iter().sum::<u8>() > 0 && bundle_contains(&ps.hand, &COST_DEV) {
                    out.push(Action::BuyDevCard);
                }

                // 海上交易
                for give in RESOURCES {
                    let rate = self.board.best_maritime_rate(p, give);
                    if ps.hand[give.idx()] < rate {
                        continue;
                    }
                    for take in RESOURCES {
                        if take != give && self.bank[take.idx()] > 0 {
                            out.push(Action::MaritimeTrade {
                                give,
                                count: rate,
                                take,
                            });
                        }
                    }
                }

                // 国内交易の提案（代表的なものだけ自動生成する）
                if self.cfg.domestic_trade
                    && self.cfg.max_generated_offers > 0
                    && self.negotiation_open()
                {
                    self.gen_offers(p, out, false);
                }
            }
        }
    }

    fn gen_dev_plays(&self, p: PlayerId, out: &mut Vec<Action>) {
        if self.dev_played_this_turn {
            return;
        }
        let ps = &self.players[p as usize];
        if ps.playable_dev(DevCard::Knight) > 0 {
            out.push(Action::PlayKnight);
        }
        if ps.playable_dev(DevCard::RoadBuilding) > 0
            && ps.roads_left > 0
            && (0..NUM_EDGES).any(|e| self.can_build_road(e as EdgeId, p))
        {
            out.push(Action::PlayRoadBuilding);
        }
        if ps.playable_dev(DevCard::Monopoly) > 0 {
            for r in RESOURCES {
                out.push(Action::PlayMonopoly(r));
            }
        }
        if ps.playable_dev(DevCard::YearOfPlenty) > 0 {
            // 銀行にある資源から 2 枚。順序は意味を持たないので i <= j に絞る
            for (i, a) in RESOURCES.iter().enumerate() {
                for b in RESOURCES.iter().skip(i) {
                    let need = if a == b { 2 } else { 1 };
                    if self.bank[a.idx()] >= need && self.bank[b.idx()] >= need {
                        out.push(Action::PlayYearOfPlenty(*a, *b));
                    }
                }
            }
        }
    }

    fn gen_robber_moves(&self, p: PlayerId, out: &mut Vec<Action>) {
        let topo = Topology::get();
        for t in 0..NUM_TILES {
            if t as TileId == self.board.robber {
                continue; // 同じ場所には置き直せない
            }
            let mut victims = [false; MAX_PLAYERS];
            let mut any = false;
            for n in topo.tile_nodes[t] {
                if let Some(b) = self.board.building[n as usize] {
                    if b.owner != p && self.players[b.owner as usize].hand_size() > 0 {
                        victims[b.owner as usize] = true;
                        any = true;
                    }
                }
            }
            if !any {
                out.push(Action::MoveRobber {
                    tile: t as TileId,
                    victim: None,
                });
            } else {
                for (q, &v) in victims.iter().enumerate().take(self.n()) {
                    if v {
                        out.push(Action::MoveRobber {
                            tile: t as TileId,
                            victim: Some(q as PlayerId),
                        });
                    }
                }
            }
        }
    }

    fn gen_discards(&self, p: PlayerId, out: &mut Vec<Action>) {
        let hand = self.players[p as usize].hand;
        let k = self.discard_pending[p as usize];
        let mut all = Vec::new();
        enumerate_bundles(&hand, k, &mut all);
        if all.len() <= self.cfg.max_discard_actions {
            out.extend(all.into_iter().map(Action::Discard));
        } else {
            // 多すぎる時は間引く。等間隔に取ることで「多い資源だけ捨てる」「均等に捨てる」
            // といった端の候補が両方残る
            let step = all.len() / self.cfg.max_discard_actions;
            for (i, b) in all.into_iter().enumerate() {
                if i % step == 0 {
                    out.push(Action::Discard(b));
                }
            }
        }
    }

    /// 代表的な交易提案（`counter` が真なら対案として）。1:1 と 2:1 の単一資源交換。
    /// 本気の交易 AI はここに頼らず自前で `OfferTrade` / `CounterOffer` を組み立てる想定。
    fn gen_offers(&self, p: PlayerId, out: &mut Vec<Action>, counter: bool) {
        let hand = self.players[p as usize].hand;
        // 対案は「相手が欲しがっていた資源」を必ず 1 枚は出す物に限る。
        //
        // ここを絞らないと、木が欲しくて出した提案に「鉄をやるから羊をくれ」と
        // まったく噛み合わない対案が返る。提案者から見れば話が繋がっておらず、
        // 断るしかない札が候補欄を埋めるだけになる。
        // 交渉は「相手の要求に応じる姿勢を見せて、こちらの取り分を釣り上げる」もの。
        let need: Bundle = if counter {
            self.trade.map(|t| t.want).unwrap_or(EMPTY)
        } else {
            EMPTY
        };
        let mut made = 0;
        let mut push = |give: Bundle, want: Bundle, made: &mut usize| -> bool {
            if *made >= self.cfg.max_generated_offers {
                return false;
            }
            // 一度断られた条件は出し直さない（対案は別の場面なので対象外）
            if !counter && self.offer_was_rejected(&give, &want) {
                return true;
            }
            // 相手の欲しい物にかすりもしない対案は作らない
            if need != EMPTY && !RESOURCES.iter().any(|r| need[r.idx()] > 0 && give[r.idx()] > 0) {
                return true;
            }
            out.push(if counter {
                Action::CounterOffer { give, want }
            } else {
                Action::OfferTrade { give, want }
            });
            *made += 1;
            true
        };

        // 1 種類 ↔ 1 種類。提案と対案で要る向きが違う:
        //  - 提案する側は「多めに出す」で相手を釣る (1:1, 2:1)
        //  - 対案を返す側は「もっと寄越せ」と値切る (1:1, 1:2)
        // ⚠ 人間は提案画面で任意の枚数を組める。ここが狭いと
        //   「人間だけが出せる条件」が生まれる（実際 1 種類↔1 種類しか無かった頃は
        //   それだけで 20pt 弱かった）。多めに積む形・値切る形まで作る。
        let ratios: [(u8, u8); 4] = if counter {
            [(1, 1), (1, 2), (1, 3), (2, 2)]
        } else {
            [(1, 1), (2, 1), (3, 1), (2, 2)]
        };
        for (gn, wn) in ratios {
            for give in RESOURCES {
                if hand[give.idx()] < gn {
                    continue;
                }
                for want in RESOURCES {
                    if want == give {
                        continue; // 同種の交換は禁止
                    }
                    if !push(bundle_of(give, gn), bundle_of(want, wn), &mut made) {
                        return;
                    }
                }
            }
        }

        if !self.cfg.wide_offers {
            return;
        }

        // 2 種類を出して 1 種類を貰う。「木と土を出すから鉄が欲しい」
        for a in RESOURCES {
            for b in RESOURCES {
                if b.idx() <= a.idx() || hand[a.idx()] < 1 || hand[b.idx()] < 1 {
                    continue;
                }
                let mut give = EMPTY;
                give[a.idx()] = 1;
                give[b.idx()] = 1;
                for want in RESOURCES {
                    if want == a || want == b {
                        continue;
                    }
                    if !push(give, bundle_of(want, 1), &mut made) {
                        return;
                    }
                }
            }
        }

        // 1 種類を出して 2 種類を貰う。建設にあと 2 種類足りない時の形
        for give in RESOURCES {
            if hand[give.idx()] < 1 {
                continue;
            }
            for c in RESOURCES {
                for d in RESOURCES {
                    if d.idx() <= c.idx() || c == give || d == give {
                        continue;
                    }
                    let mut want = EMPTY;
                    want[c.idx()] = 1;
                    want[d.idx()] = 1;
                    if !push(bundle_of(give, 1), want, &mut made) {
                        return;
                    }
                }
            }
        }
    }

    /// その条件は、この手番で既に断られているか
    pub fn offer_was_rejected(&self, give: &Bundle, want: &Bundle) -> bool {
        match canonical_offer_index(give, want) {
            Some(i) => self.rejected_offers[(i / 128) as usize] & (1u128 << (i % 128)) != 0,
            None => false,
        }
    }

    fn mark_offer_rejected(&mut self, give: &Bundle, want: &Bundle) {
        if let Some(i) = canonical_offer_index(give, want) {
            self.rejected_offers[(i / 128) as usize] |= 1u128 << (i % 128);
        }
    }

    /// 交易の提案・対案として成立しうる形か。公式の禁止事項を検査する。
    fn validate_offer(&self, from: PlayerId, give: &Bundle, want: &Bundle) {
        assert!(
            bundle_contains(&self.players[from as usize].hand, give),
            "持っていない資源を出そうとした"
        );
        assert!(
            bundle_total(give) > 0 && bundle_total(want) > 0,
            "一方的な譲渡はできない"
        );
        assert!(
            (0..NUM_RESOURCES).all(|i| give[i] == 0 || want[i] == 0),
            "同種の資源を交換することはできない"
        );
    }

    /// `from` が `give` を出し、`to` が `want` を出す形で交換する。
    fn settle_trade(&mut self, from: PlayerId, to: PlayerId, give: &Bundle, want: &Bundle) {
        for i in 0..NUM_RESOURCES {
            debug_assert!(self.players[from as usize].hand[i] >= give[i]);
            debug_assert!(self.players[to as usize].hand[i] >= want[i]);
            self.players[from as usize].hand[i] -= give[i];
            self.players[to as usize].hand[i] += give[i];
            self.players[to as usize].hand[i] -= want[i];
            self.players[from as usize].hand[i] += want[i];
        }
    }

    // =====================================================================
    // 配置の可否
    // =====================================================================

    /// 距離ルール: 隣接する 3 頂点すべてが空でなければ建てられない（自分の建物でも不可）
    pub fn can_place_settlement(&self, n: NodeId, p: PlayerId, initial: bool) -> bool {
        let topo = Topology::get();
        if self.board.building[n as usize].is_some() {
            return false;
        }
        for m in &topo.node_neighbors[n as usize] {
            if self.board.building[m as usize].is_some() {
                return false;
            }
        }
        if initial {
            return true;
        }
        // 通常の建設は自分の道に接している必要がある
        topo.node_edges[n as usize]
            .into_iter()
            .any(|e| self.board.road[e as usize] == Some(p))
    }

    /// 道を敷けるか。相手の建物がある頂点からは先へ伸ばせない（公式 Illustration K）。
    pub fn can_build_road(&self, e: EdgeId, p: PlayerId) -> bool {
        let topo = Topology::get();
        if self.board.road[e as usize].is_some() {
            return false;
        }
        for n in topo.edge_nodes[e as usize] {
            match self.board.building[n as usize] {
                // 自分の建物からは伸ばせる
                Some(b) if b.owner == p => return true,
                // 相手の建物では止まる
                Some(_) => continue,
                None => {
                    // 空の頂点なら、自分の道が接していれば伸ばせる
                    if topo.node_edges[n as usize]
                        .into_iter()
                        .any(|f| f != e && self.board.road[f as usize] == Some(p))
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    // =====================================================================
    // 行動の適用
    // =====================================================================

    /// その手を **今この局面で適用してよいか**。
    ///
    /// `apply` は非合法手で panic するので、外から手が来る場所（オンライン対戦のサーバ）では
    /// 必ずここを先に通す。release では `panic = "abort"` なので、
    /// 撥ねそこねると process ごと落ちる。
    pub fn can_apply(&self, a: &Action) -> bool {
        if self.winner.is_some() {
            return false;
        }
        match a {
            Action::Discard(b) => {
                matches!(self.prompt, Prompt::Discard)
                    && bundle_total(b) as usize == self.discard_pending[self.to_act as usize] as usize
                    && bundle_contains(&self.players[self.to_act as usize].hand, b)
            }
            Action::OfferTrade { give, want } | Action::CounterOffer { give, want }
            | Action::CounterAlt { give, want } => {
                let ok_shape = bundle_total(give) > 0
                    && bundle_total(want) > 0
                    && (0..NUM_RESOURCES).all(|i| give[i] == 0 || want[i] == 0)
                    && bundle_contains(&self.players[self.to_act as usize].hand, give);
                if !ok_shape || !self.negotiation_open() {
                    return false;
                }
                match a {
                    Action::OfferTrade { .. } => {
                        matches!(self.prompt, Prompt::PlayTurn) && self.cfg.domestic_trade && self.rolled
                    }
                    _ => matches!(self.prompt, Prompt::DecideTrade),
                }
            }
            Action::CounterAltRemove(_) => {
                matches!(self.prompt, Prompt::DecideTrade) && self.negotiation_open()
            }
            other => self.legal_actions().contains(other),
        }
    }

    pub fn apply(&mut self, a: Action) -> ActionRecord {
        self.apply_forced(a, Forced::No)
    }

    /// 偶然の結果を外から与えて適用する（expectimax の分岐展開用）。
    pub fn apply_forced(&mut self, a: Action, forced: Forced) -> ActionRecord {
        debug_assert!(
            self.legal_actions().contains(&a) || matches!(a, Action::Discard(_) | Action::OfferTrade { .. } | Action::CounterOffer { .. } | Action::CounterAlt { .. } | Action::CounterAltRemove(_)),
            "非合法手が渡された: {a:?} (prompt={:?})",
            self.prompt
        );
        let actor = self.to_act;
        let mut result = Outcome::None;

        // 合法性は「その行動を選んだ時点の時計」で判定する。
        // 先に時計を進めてしまうと、legal_actions が提案を出した直後に
        // apply が「時間切れ」と言い出す 1 手ぶんのズレが生まれる。
        let negotiation_was_open = self.negotiation_open();
        // 仮想時計はここで進める。EndTurn の中でリセットされるので、
        // 「手番の最後の 1 手が次の手番の予算を食う」ことにはならない。
        if let TurnClock::PerAction { ms } = self.cfg.turn_clock {
            self.turn_elapsed_ms = self.turn_elapsed_ms.saturating_add(ms);
        }

        match a {
            Action::SetupSettlement(n) => {
                self.place_building(n, actor, BuildingKind::Settlement);
                self.players[actor as usize].settlements_left -= 1;
                // 2 巡目の開拓地からは資源をもらう
                if self.setup_index >= self.num_players {
                    let topo = Topology::get();
                    for t in &topo.node_tiles[n as usize] {
                        if let Some(r) = self.board.tile_resource[t as usize] {
                            self.take_from_bank(actor, r, 1);
                        }
                    }
                }
                self.last_setup_node = n;
                self.prompt = Prompt::SetupRoad;
            }

            Action::SetupRoad(e) => {
                self.board.road[e as usize] = Some(actor);
                self.players[actor as usize].roads_left -= 1;
                self.recompute_longest_road();
                self.setup_index += 1;
                // 初期配置も 1 人ずつの「手番」なので持ち時間はそこで区切る
                self.turn_elapsed_ms = 0;
                if self.is_setup() {
                    self.to_act = self.setup_actor(self.setup_index);
                    self.prompt = Prompt::SetupSettlement;
                } else {
                    // 初期配置終了。最後に 2 軒目を置いた開始プレイヤーから
                    self.turn_player = 0;
                    self.to_act = 0;
                    self.turn = 1;
                    self.prompt = Prompt::PlayTurn;
                }
            }

            Action::Roll => {
                let (d1, d2) = match forced {
                    Forced::Dice(a, b) => (a, b),
                    _ => self.rng_dice.dice(),
                };
                result = Outcome::Dice(d1, d2);
                self.rolled = true;
                let sum = d1 + d2;
                if sum == 7 {
                    self.begin_seven();
                } else {
                    self.produce(sum);
                }
            }

            Action::Discard(b) => {
                let ps = &mut self.players[actor as usize];
                for i in 0..NUM_RESOURCES {
                    debug_assert!(ps.hand[i] >= b[i]);
                    ps.hand[i] -= b[i];
                    self.bank[i] += b[i];
                }
                self.discard_pending[actor as usize] = 0;
                self.advance_discard();
            }

            Action::MoveRobber { tile, victim } => {
                self.board.robber = tile;
                let stolen = match (victim, forced) {
                    (None, _) => None,
                    (Some(v), Forced::Steal(s)) => {
                        if let Some(r) = s {
                            self.move_card(v, actor, r);
                        }
                        s
                    }
                    (Some(v), _) => {
                        let r = self.steal_random(v, actor);
                        r
                    }
                };
                result = Outcome::Stole(stolen);
                self.prompt = Prompt::PlayTurn;
                self.to_act = self.turn_player;
            }

            Action::BuildRoad(e) => {
                if self.free_roads > 0 {
                    self.free_roads -= 1;
                } else {
                    self.pay(actor, &COST_ROAD);
                }
                self.board.road[e as usize] = Some(actor);
                self.players[actor as usize].roads_left -= 1;
                self.recompute_longest_road();
            }

            Action::BuildSettlement(n) => {
                self.pay(actor, &COST_SETTLEMENT);
                self.place_building(n, actor, BuildingKind::Settlement);
                self.players[actor as usize].settlements_left -= 1;
                // 相手の道を分断した可能性があるので測り直す
                self.recompute_longest_road();
            }

            Action::BuildCity(n) => {
                self.pay(actor, &COST_CITY);
                self.board.building[n as usize] = Some(Building {
                    owner: actor,
                    kind: BuildingKind::City,
                });
                let ps = &mut self.players[actor as usize];
                ps.cities_left -= 1;
                ps.settlements_left += 1; // 開拓地の駒は手元に戻る
            }

            Action::BuyDevCard => {
                self.pay(actor, &COST_DEV);
                let card = match forced {
                    Forced::Draw(c) => c,
                    _ => self.draw_dev(),
                };
                debug_assert!(self.dev_deck[card.idx()] > 0);
                self.dev_deck[card.idx()] -= 1;
                let ps = &mut self.players[actor as usize];
                ps.dev[card.idx()] += 1;
                ps.dev_bought_this_turn[card.idx()] += 1;
                result = Outcome::Drew(card);
            }

            Action::PlayKnight => {
                self.consume_dev(actor, DevCard::Knight);
                self.update_largest_army(actor);
                self.prompt = Prompt::MoveRobber;
            }

            Action::PlayRoadBuilding => {
                self.consume_dev(actor, DevCard::RoadBuilding);
                // 「今置ける本数」で上限を決めてはいけない。1 本置くと隣が新たに置けるようになるため。
                // 置き場が尽きた場合は normalize() が打ち切る。
                self.free_roads = 2u8.min(self.players[actor as usize].roads_left);
                self.prompt = Prompt::FreeRoad;
            }

            Action::PlayYearOfPlenty(a1, a2) => {
                self.consume_dev(actor, DevCard::YearOfPlenty);
                self.take_from_bank(actor, a1, 1);
                self.take_from_bank(actor, a2, 1);
            }

            Action::PlayMonopoly(r) => {
                self.consume_dev(actor, DevCard::Monopoly);
                let mut total = 0;
                for q in 0..self.n() {
                    if q as PlayerId == actor {
                        continue;
                    }
                    total += self.players[q].hand[r.idx()];
                    self.players[q].hand[r.idx()] = 0;
                }
                self.players[actor as usize].hand[r.idx()] += total;
            }

            Action::MaritimeTrade { give, count, take } => {
                let ps = &mut self.players[actor as usize];
                ps.hand[give.idx()] -= count;
                self.bank[give.idx()] += count;
                self.take_from_bank(actor, take, 1);
            }

            Action::OfferTrade { give, want } => {
                // 自動生成された提案以外（AI が自前で組んだもの）もここを通るので検証する。
                assert!(negotiation_was_open, "手番の持ち時間が切れている");
                self.validate_offer(actor, &give, &want);
                self.offers_this_turn += 1;
                self.trade = Some(TradeState {
                    proposer: actor,
                    give,
                    want,
                    responder: self.next_player(actor),
                    accepted: [false; MAX_PLAYERS],
                    counters: [[None; MAX_COUNTER_ALTS]; MAX_PLAYERS],
                    responded: [false; MAX_PLAYERS],
                    answered: 0,
                });
                // 最初に聞く相手。人間は最後に回す（`answer_last_mask`）
                let first = {
                    let t = self.trade.expect("いま入れた");
                    self.next_responder(&t).expect("提案には必ず相手がいる")
                };
                if let Some(t) = self.trade.as_mut() {
                    t.responder = first;
                }
                self.prompt = Prompt::DecideTrade;
                self.to_act = first;
            }

            // 積んだ候補を取り消す。返答はまだ終わっていないので、いつでも戻せる
            Action::CounterAltRemove(i) => {
                let mut t = self.trade.expect("交易中でない");
                let slots = &mut t.counters[actor as usize];
                if (i as usize) < slots.len() {
                    slots[i as usize] = None;
                    // 空きが飛び飛びにならないよう前へ詰める（UI の番号と一致させる）
                    let mut kept = [None; MAX_COUNTER_ALTS];
                    let mut k = 0;
                    for c in slots.iter() {
                        if let Some(v) = c {
                            kept[k] = Some(*v);
                            k += 1;
                        }
                    }
                    *slots = kept;
                }
                self.trade = Some(t);
            }

            // 候補を積むだけ。返答はまだ終わらないので、手番も返答数も動かさない
            Action::CounterAlt { give, want } => {
                assert!(negotiation_was_open, "手番の持ち時間が切れている");
                self.validate_offer(actor, &give, &want);
                let mut t = self.trade.expect("交易中でない");
                push_counter(&mut t.counters[actor as usize], give, want);
                self.trade = Some(t);
            }

            Action::AcceptTrade | Action::RejectTrade | Action::CounterOffer { .. } => {
                let mut t = self.trade.expect("交易中でない");
                match a {
                    Action::AcceptTrade => t.accepted[actor as usize] = true,
                    Action::CounterOffer { give, want } => {
                        assert!(negotiation_was_open, "手番の持ち時間が切れている");
                        self.validate_offer(actor, &give, &want);
                        push_counter(&mut t.counters[actor as usize], give, want);
                    }
                    _ => {}
                }
                t.responded[actor as usize] = true;
                t.answered += 1;
                // 全員の返答を待ってから提案者が相手を選ぶ。
                // 「最初に承諾した人と成立」にすると、速いだけの相手が得をして
                // 戦略の評価が歪む（Guhe & Lascarides 2014 が実測した落とし穴）
                if t.answered as usize == self.n() - 1 {
                    // 承諾が 1 つでも、対案が 1 つでもあれば提案者が選ぶ場面へ進む
                    let has_any = t.accepted.iter().any(|&x| x)
                        || t.counters.iter().any(|cs| cs.iter().any(|c| c.is_some()));
                    self.to_act = t.proposer;
                    if has_any {
                        self.trade = Some(t);
                        self.prompt = Prompt::DecideAcceptees;
                    } else {
                        // 誰も乗らなかった条件は、この手番ではもう出さない
                        self.mark_offer_rejected(&t.give, &t.want);
                        self.trade = None;
                        self.prompt = Prompt::PlayTurn;
                    }
                } else {
                    // まだ答えていない相手のうち、人間でない席を先に回す
                    let nxt = self.next_responder(&t).expect("残りがいるはず");
                    t.responder = nxt;
                    self.trade = Some(t);
                    self.to_act = nxt;
                }
            }

            Action::ConfirmTrade(partner) => {
                let t = self.trade.take().expect("交易中でない");
                debug_assert!(t.accepted[partner as usize], "承諾していない相手と成立させた");
                self.settle_trade(t.proposer, partner, &t.give, &t.want);
                self.prompt = Prompt::PlayTurn;
                self.to_act = self.turn_player;
            }

            Action::AcceptCounter { from: partner, alt } => {
                let t = self.trade.take().expect("交易中でない");
                let (cgive, cwant) =
                    t.counters[partner as usize][alt as usize].expect("その候補は無い");
                // 向きは対案を出した側から見たもの: partner が cgive を出し、提案者が cwant を出す
                self.settle_trade(partner, t.proposer, &cgive, &cwant);
                self.prompt = Prompt::PlayTurn;
                self.to_act = self.turn_player;
            }

            Action::CancelTrade => {
                // 自分で下げた条件も出し直さない（同じ答えしか返ってこない）
                if let Some(t) = self.trade {
                    self.mark_offer_rejected(&t.give, &t.want);
                }
                self.trade = None;
                self.prompt = Prompt::PlayTurn;
                self.to_act = self.turn_player;
            }

            Action::EndTurn => {
                let ps = &mut self.players[self.turn_player as usize];
                ps.dev_bought_this_turn = [0; NUM_DEV_KINDS];
                self.rolled = false;
                self.dev_played_this_turn = false;
                self.free_roads = 0;
                self.offers_this_turn = 0;
                self.rejected_offers = [0; 2];
                self.turn_elapsed_ms = 0;
                self.turn_player = self.next_player(self.turn_player);
                self.to_act = self.turn_player;
                self.prompt = Prompt::PlayTurn;
                self.turn += 1;
            }
        }

        self.normalize();
        self.check_win();

        ActionRecord {
            actor,
            action: a,
            result,
        }
    }

    // ---------------------------------------------------------------- 補助

    fn place_building(&mut self, n: NodeId, p: PlayerId, kind: BuildingKind) {
        debug_assert!(self.board.building[n as usize].is_none());
        self.board.building[n as usize] = Some(Building { owner: p, kind });
    }

    fn pay(&mut self, p: PlayerId, cost: &Bundle) {
        let ps = &mut self.players[p as usize];
        for i in 0..NUM_RESOURCES {
            debug_assert!(ps.hand[i] >= cost[i], "支払えない");
            ps.hand[i] -= cost[i];
            self.bank[i] += cost[i];
        }
    }

    fn take_from_bank(&mut self, p: PlayerId, r: Resource, n: u8) {
        let take = n.min(self.bank[r.idx()]);
        self.bank[r.idx()] -= take;
        self.players[p as usize].hand[r.idx()] += take;
    }

    fn move_card(&mut self, from: PlayerId, to: PlayerId, r: Resource) {
        debug_assert!(self.players[from as usize].hand[r.idx()] > 0);
        self.players[from as usize].hand[r.idx()] -= 1;
        self.players[to as usize].hand[r.idx()] += 1;
    }

    fn steal_random(&mut self, from: PlayerId, to: PlayerId) -> Option<Resource> {
        let total = self.players[from as usize].hand_size();
        if total == 0 {
            return None;
        }
        let mut k = self.rng_steal.below(total as u32) as u8;
        for r in RESOURCES {
            let c = self.players[from as usize].hand[r.idx()];
            if k < c {
                self.move_card(from, to, r);
                return Some(r);
            }
            k -= c;
        }
        unreachable!("手札の走査が外れた")
    }

    fn draw_dev(&mut self) -> DevCard {
        let total: u8 = self.dev_deck.iter().sum();
        debug_assert!(total > 0);
        let mut k = self.rng_dev.below(total as u32) as u8;
        for c in DEV_CARDS {
            let n = self.dev_deck[c.idx()];
            if k < n {
                return c;
            }
            k -= n;
        }
        unreachable!("山札の走査が外れた")
    }

    fn consume_dev(&mut self, p: PlayerId, c: DevCard) {
        let ps = &mut self.players[p as usize];
        debug_assert!(ps.playable_dev(c) > 0, "使えない発展カード");
        ps.dev[c.idx()] -= 1;
        ps.played_dev[c.idx()] += 1;
        self.dev_played_this_turn = true;
    }

    /// 資源産出。銀行が足りない場合の公式ルールを含む。
    fn produce(&mut self, roll: u8) {
        let topo = Topology::get();
        let mut demand = [[0u8; NUM_RESOURCES]; MAX_PLAYERS];
        for t in 0..NUM_TILES {
            if self.board.tile_number[t] != roll || t as TileId == self.board.robber {
                continue;
            }
            let Some(res) = self.board.tile_resource[t] else {
                continue;
            };
            for n in topo.tile_nodes[t] {
                if let Some(b) = self.board.building[n as usize] {
                    let amount = match b.kind {
                        BuildingKind::Settlement => 1,
                        BuildingKind::City => 2,
                    };
                    demand[b.owner as usize][res.idx()] += amount;
                }
            }
        }

        for r in 0..NUM_RESOURCES {
            let total: u16 = (0..self.n()).map(|q| demand[q][r] as u16).sum();
            if total == 0 {
                continue;
            }
            let claimants = (0..self.n()).filter(|&q| demand[q][r] > 0).count();
            if total <= self.bank[r] as u16 {
                for q in 0..self.n() {
                    self.players[q].hand[r] += demand[q][r];
                    self.bank[r] -= demand[q][r];
                }
            } else if claimants == 1 {
                // 影響が 1 人だけなら、残っている分だけ渡す（端数は消滅）
                let q = (0..self.n()).find(|&q| demand[q][r] > 0).unwrap();
                let give = self.bank[r];
                self.players[q].hand[r] += give;
                self.bank[r] = 0;
            }
            // それ以外は誰も受け取らない
        }
    }

    fn begin_seven(&mut self) {
        for q in 0..self.n() {
            let h = self.players[q].hand_size();
            self.discard_pending[q] = if h > HAND_LIMIT { h / 2 } else { 0 };
        }
        self.advance_discard();
    }

    /// 次に捨てるべきプレイヤーへ進む。全員終わったら盗賊の移動へ。
    fn advance_discard(&mut self) {
        for k in 0..self.n() {
            let q = (self.turn_player as usize + k) % self.n();
            if self.discard_pending[q] > 0 {
                self.to_act = q as PlayerId;
                self.prompt = Prompt::Discard;
                return;
            }
        }
        self.to_act = self.turn_player;
        self.prompt = Prompt::MoveRobber;
    }

    fn recompute_longest_road(&mut self) {
        let mut lens = [0u8; MAX_PLAYERS];
        for q in 0..self.n() {
            let l = longest_road_length(&self.board, q as PlayerId);
            self.players[q].longest_road = l;
            lens[q] = l;
        }
        self.longest_road_owner = resolve_longest_road(&lens[..self.n()], self.longest_road_owner);
    }

    fn update_largest_army(&mut self, p: PlayerId) {
        let mine = self.players[p as usize].played_knights();
        if mine < MIN_ARMY {
            return;
        }
        match self.largest_army_owner {
            None => self.largest_army_owner = Some(p),
            Some(cur) if cur != p && mine > self.players[cur as usize].played_knights() => {
                self.largest_army_owner = Some(p)
            }
            _ => {}
        }
    }

    /// 手が無い状態を潰す（無償道が置けない、廃棄対象が居ない、など）
    fn normalize(&mut self) {
        if self.prompt == Prompt::FreeRoad {
            let has = (0..NUM_EDGES).any(|e| self.can_build_road(e as EdgeId, self.to_act));
            if self.free_roads == 0 || !has || self.players[self.to_act as usize].roads_left == 0 {
                self.free_roads = 0;
                self.prompt = Prompt::PlayTurn;
                self.to_act = self.turn_player;
            }
        }
    }

    fn check_win(&mut self) {
        if self.is_setup() || self.winner.is_some() {
            return;
        }
        // 勝利宣言できるのは自分の手番だけ（公式）
        if self.actual_vp(self.turn_player) >= VP_TO_WIN {
            self.winner = Some(self.turn_player);
            self.prompt = Prompt::GameOver;
        }
    }
}

/// 単一資源どうしの提案を 0..75 の番号にする。それ以外は `None`。
///
/// `legal_actions` が自動生成する提案は「1:1 / 2:1 / 1:2 の単一資源」だけなので、
/// この番号で「もう断られた」を記録できる。
pub fn canonical_offer_index(give: &Bundle, want: &Bundle) -> Option<u8> {
    // 自動生成する提案の形は 8 系統・合計 180 通り。[u128; 2] のビットに収まる。
    //   0..20   1種類1枚 → 1種類1枚
    //  20..40   1種類2枚 → 1種類1枚
    //  40..60   1種類1枚 → 1種類2枚
    //  60..90   2種類1枚ずつ → 1種類1枚
    //  90..120  1種類1枚 → 2種類1枚ずつ
    // 120..140  1種類3枚 → 1種類1枚   ← 人間だけが出せていた「多めに積む」形
    // 140..160  1種類1枚 → 1種類3枚
    // 160..180  1種類2枚 → 1種類2枚
    let kinds = |b: &Bundle| -> Vec<(usize, u8)> {
        (0..NUM_RESOURCES).filter(|&i| b[i] > 0).map(|i| (i, b[i])).collect()
    };
    let g = kinds(give);
    let w = kinds(want);

    // 5 種類のうち `skip` を除いた並びでの `i` の順位
    let rank1 = |i: usize, skip: usize| -> u8 {
        (0..NUM_RESOURCES).filter(|&k| k != skip).position(|k| k == i).unwrap() as u8
    };
    // 5 種類から 2 つ選ぶ組み合わせの通し番号（0..10）
    let pair2 = |a: usize, b: usize| -> u8 {
        let mut n = 0u8;
        for x in 0..NUM_RESOURCES {
            for y in (x + 1)..NUM_RESOURCES {
                if x == a && y == b {
                    return n;
                }
                n += 1;
            }
        }
        u8::MAX
    };
    // `skip` を除いた 4 種類から 2 つ選ぶ組み合わせの通し番号（0..6）
    let pair_wo = |a: usize, b: usize, skip: usize| -> u8 {
        let rest: Vec<usize> = (0..NUM_RESOURCES).filter(|&k| k != skip).collect();
        let mut n = 0u8;
        for x in 0..rest.len() {
            for y in (x + 1)..rest.len() {
                if rest[x] == a && rest[y] == b {
                    return n;
                }
                n += 1;
            }
        }
        u8::MAX
    };

    match (g.as_slice(), w.as_slice()) {
        (&[(gi, gn)], &[(wi, wn)]) => {
            let base = match (gn, wn) {
                (1, 1) => 0,
                (2, 1) => 20,
                (1, 2) => 40,
                (3, 1) => 120,
                (1, 3) => 140,
                (2, 2) => 160,
                _ => return None,
            };
            Some(base + gi as u8 * 4 + rank1(wi, gi))
        }
        (&[(a, 1), (b, 1)], &[(wi, 1)]) => {
            let p = pair2(a, b);
            // want は a・b 以外の 3 種類のどれか
            let rest: Vec<usize> = (0..NUM_RESOURCES).filter(|&k| k != a && k != b).collect();
            let r = rest.iter().position(|&k| k == wi)? as u8;
            Some(60 + p * 3 + r)
        }
        (&[(gi, 1)], &[(c, 1), (d, 1)]) => {
            let p = pair_wo(c, d, gi);
            if p == u8::MAX {
                return None;
            }
            Some(90 + gi as u8 * 6 + p)
        }
        _ => None,
    }
}

/// 64bit の種を混ぜる（SplitMix64）。1 本の種から独立な流れを作るのに使う。
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 対案の候補を空いている枠に積む。同じ条件は重ねない。埋まっていれば捨てる。
fn push_counter(slots: &mut [Option<(Bundle, Bundle)>; MAX_COUNTER_ALTS], give: Bundle, want: Bundle) {
    if slots.iter().any(|c| *c == Some((give, want))) {
        return;
    }
    if let Some(free) = slots.iter_mut().find(|c| c.is_none()) {
        *free = Some((give, want));
    }
}

/// `hand` から合計 `k` 枚を選ぶ全ての内訳を列挙する。
pub fn enumerate_bundles(hand: &Bundle, k: u8, out: &mut Vec<Bundle>) {
    fn rec(hand: &Bundle, i: usize, left: u8, cur: &mut Bundle, out: &mut Vec<Bundle>) {
        if i == NUM_RESOURCES {
            if left == 0 {
                out.push(*cur);
            }
            return;
        }
        let remaining_capacity: u8 = hand[i..].iter().sum();
        if remaining_capacity < left {
            return;
        }
        let hi = left.min(hand[i]);
        for take in 0..=hi {
            cur[i] = take;
            rec(hand, i + 1, left - take, cur, out);
        }
        cur[i] = 0;
    }
    let mut cur = EMPTY;
    rec(hand, 0, k, &mut cur, out);
}
