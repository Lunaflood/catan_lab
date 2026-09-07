//! 初期配置の評価関数。
//!
//! 調査で最も効き目が大きかった項目（配置ロジックだけを変えて勝率 15.2% → 28.3%:
//! Guhe & Lascarides 2014, 各条件 10,000 戦）。
//!
//! 特徴量は「なぜそれが効くのか」を説明できるものだけにしてある。
//! 重みは暫定値で、**必ず ablation（項目を 1 つずつ切って測る）で検証する**。
//! 手で付けた重みが素のランダムより弱くなった前例がある（Szita et al. 2010）。

use catan_core::board::{Board, BuildingKind, PlayerId, PortKind, Resource, NUM_RESOURCES};
use catan_core::topology::{EdgeId, NodeId, Topology, NUM_NODES};
use catan_core::view::View;

/// 道 1 本 + 開拓地（次の 1 軒を建てるまでの総コスト）
const COST_EXPAND: [u8; NUM_RESOURCES] = [2, 2, 1, 1, 0];
const COST_CITY: [u8; NUM_RESOURCES] = [0, 0, 0, 2, 3];
const COST_DEV: [u8; NUM_RESOURCES] = [0, 0, 1, 1, 1];

#[derive(Clone, Copy, Debug)]
pub struct PlacementWeights {
    /// 総 pip。「その場所がどれだけ産出するか」の一次近似
    pub pip: f32,
    /// まだ持っていない資源を 1 種増やすことの価値。
    /// catanatron は「新資源 1 種 = 産出 4 pip 相当」と換算している
    pub variety_new: f32,
    /// 既に持っている数字と重複することの罰。
    /// 産出が散っている方が「毎ターン何かが入る」確率が上がり、7 での廃棄も減る
    pub number_dup: f32,
    /// 既に接しているヘクスと重複することの罰（数字重複の特に悪い場合）
    pub same_hex: f32,
    /// 拡張（道+開拓地）の速さ = 1 / 推定建設ターン数
    pub etb_expand: f32,
    /// 都市化の速さ
    pub etb_city: f32,
    /// 発展カードの速さ
    pub etb_dev: f32,
    /// 産出とかみ合う 2:1 港
    pub port_specific: f32,
    /// 3:1 港
    pub port_generic: f32,
    /// 2 歩で届く空き頂点の数（囲まれないこと）
    pub expansion: f32,
}

impl Default for PlacementWeights {
    /// 実測で残った重み。各条件 10,000 戦 × 席順ローテーション、相手は WeightedRandom×3。
    ///
    /// 最初は 10 項目すべてに手で重みを付けたが、ablation で**効いていたのは 2 項目だけ**だった:
    /// - `pip` (+10.1pt) と `variety_new` (+7.9pt) だけが有意
    /// - `number_dup` (-1.0pt) と `same_hex` (-0.6pt) は**入れると弱くなった**
    /// - ETB 3 項目・港 2 項目・拡張余地はいずれも測れる効果なし
    ///   （ETB は産出と強く相関していて独立な情報を持たない）
    ///
    /// ⚠ この結論は「配置以外が WeightedRandom」という前提の下でのもの。
    /// 下流の方策が強くなったら、落とした項目は**全部測り直すこと**。
    /// 産出の散らしや港は、計画的に打てる打ち手を持って初めて価値が出る種類の特徴量。
    fn default() -> Self {
        PlacementWeights {
            pip: 1.0,
            // 4 以上は頭打ち（4/6/8/12/16 がすべて 75.9〜76.0%）。中央を取る
            variety_new: 5.0,
            number_dup: 0.0,
            same_hex: 0.0,
            etb_expand: 0.0,
            etb_city: 0.0,
            etb_dev: 0.0,
            port_specific: 0.0,
            port_generic: 0.0,
            expansion: 0.0,
        }
    }
}

impl PlacementWeights {
    /// 最初に手で決めた重み。ablation の出発点であり、「手で決めるとこうなる」の記録。
    pub fn handmade() -> Self {
        PlacementWeights {
            pip: 1.0,
            variety_new: 4.0,
            number_dup: -0.5,
            same_hex: -3.0,
            etb_expand: 8.0,
            etb_city: 6.0,
            etb_dev: 3.0,
            port_specific: 3.0,
            port_generic: 1.5,
            expansion: 0.6,
        }
    }
}

impl PlacementWeights {
    /// 自動調整で出した初期配置の重み（2026-09-07）。
    ///
    /// ⚠ 評価関数側ほど確かではない。掃引中に `pip` と `variety_new` が
    /// 巡ごとに行ったり来たりしていて（1.0↔2.7 / 4.0↔1.4）、誤差を拾っている疑いがある。
    /// **既定に戻した版と最終検証で比べて、勝った方をここに置く**こと。
    pub fn tuned() -> Self {
        PlacementWeights {
            pip: 2.744,
            variety_new: 1.372,
            expansion: 0.3,
            ..PlacementWeights::default()
        }
    }
}

impl PlacementWeights {
    /// ablation 用。指定した項目だけ 0 にする。
    pub fn without(mut self, name: &str) -> Self {
        match name {
            "pip" => self.pip = 0.0,
            "variety_new" => self.variety_new = 0.0,
            "number_dup" => self.number_dup = 0.0,
            "same_hex" => self.same_hex = 0.0,
            "etb_expand" => self.etb_expand = 0.0,
            "etb_city" => self.etb_city = 0.0,
            "etb_dev" => self.etb_dev = 0.0,
            "port_specific" => self.port_specific = 0.0,
            "port_generic" => self.port_generic = 0.0,
            "expansion" => self.expansion = 0.0,
            other => panic!("知らない項目: {other}"),
        }
        self
    }

    /// 名前で 1 項目だけ差し替える（重みの自動調整に使う）
    pub fn set(&mut self, name: &str, v: f32) {
        match name {
            "pip" => self.pip = v,
            "variety_new" => self.variety_new = v,
            "number_dup" => self.number_dup = v,
            "same_hex" => self.same_hex = v,
            "etb_expand" => self.etb_expand = v,
            "etb_city" => self.etb_city = v,
            "etb_dev" => self.etb_dev = v,
            "port_specific" => self.port_specific = v,
            "port_generic" => self.port_generic = v,
            "expansion" => self.expansion = v,
            other => panic!("知らない項目: {other}"),
        }
    }

    /// 名前で 1 項目を読む
    pub fn get(&self, name: &str) -> f32 {
        match name {
            "pip" => self.pip,
            "variety_new" => self.variety_new,
            "number_dup" => self.number_dup,
            "same_hex" => self.same_hex,
            "etb_expand" => self.etb_expand,
            "etb_city" => self.etb_city,
            "etb_dev" => self.etb_dev,
            "port_specific" => self.port_specific,
            "port_generic" => self.port_generic,
            "expansion" => self.expansion,
            other => panic!("知らない項目: {other}"),
        }
    }

    pub const ALL_TERMS: [&'static str; 10] = [
        "pip",
        "variety_new",
        "number_dup",
        "same_hex",
        "etb_expand",
        "etb_city",
        "etb_dev",
        "port_specific",
        "port_generic",
        "expansion",
    ];
}

/// 自分の建物が生む資源別 pip（都市は 2 倍）
pub fn my_production(board: &Board, me: PlayerId) -> [u16; NUM_RESOURCES] {
    let topo = Topology::get();
    let mut out = [0u16; NUM_RESOURCES];
    for n in 0..NUM_NODES {
        let Some(b) = board.building[n] else { continue };
        if b.owner != me {
            continue;
        }
        let mult: u16 = match b.kind {
            BuildingKind::Settlement => 1,
            BuildingKind::City => 2,
        };
        for t in &topo.node_tiles[n] {
            if let Some(r) = board.tile_resource[t as usize] {
                out[r.idx()] += mult * board.tile_pips(t) as u16;
            }
        }
    }
    out
}

/// 自分が接している数字ごとの枚数（2..=12）
fn my_numbers(board: &Board, me: PlayerId) -> [u8; 13] {
    let topo = Topology::get();
    let mut out = [0u8; 13];
    for n in 0..NUM_NODES {
        let Some(b) = board.building[n] else { continue };
        if b.owner != me {
            continue;
        }
        for t in &topo.node_tiles[n] {
            let num = board.tile_number[t as usize] as usize;
            if num > 0 {
                out[num] += 1;
            }
        }
    }
    out
}

/// 自分が接しているヘクス
fn my_tiles(board: &Board, me: PlayerId) -> [bool; catan_core::topology::NUM_TILES] {
    let topo = Topology::get();
    let mut out = [false; catan_core::topology::NUM_TILES];
    for n in 0..NUM_NODES {
        let Some(b) = board.building[n] else { continue };
        if b.owner != me {
            continue;
        }
        for t in &topo.node_tiles[n] {
            out[t as usize] = true;
        }
    }
    out
}

/// 資源ごとの「1 ターンあたり実質何枚入るか」。
/// 産出が無い資源も、余っている資源を港/銀行で換えれば手に入るので、その分を足す。
fn effective_rates(pips: [u16; NUM_RESOURCES], ports: &[PortKind]) -> [f32; NUM_RESOURCES] {
    let rate: [f32; NUM_RESOURCES] = std::array::from_fn(|i| pips[i] as f32 / 36.0);
    let total: f32 = rate.iter().sum();
    std::array::from_fn(|i| {
        let r = Resource::from_idx(i);
        let mut exch = 4.0f32;
        for p in ports {
            exch = exch.min(p.rate_for(r) as f32);
        }
        // 自前の産出 + 余りを交換して得られる分
        rate[i] + (total - rate[i]) / exch
    })
}

/// コスト `cost` を貯めるのにかかる推定ターン数。無理なら `f32::INFINITY`。
fn etb(cost: &[u8; NUM_RESOURCES], eff: &[f32; NUM_RESOURCES]) -> f32 {
    let mut worst = 0.0f32;
    for i in 0..NUM_RESOURCES {
        if cost[i] == 0 {
            continue;
        }
        if eff[i] <= 1e-6 {
            return f32::INFINITY;
        }
        // 並行して貯まるので、いちばん遅い資源が律速
        worst = worst.max(cost[i] as f32 / eff[i]);
    }
    worst
}

/// 速さの寄与。ETB が短いほど大きい。青天井にならないよう頭を押さえる。
fn speed(cost: &[u8; NUM_RESOURCES], eff: &[f32; NUM_RESOURCES]) -> f32 {
    let t = etb(cost, eff);
    if t.is_finite() {
        (1.0 / t.max(1.0)).min(1.0)
    } else {
        0.0
    }
}

/// 距離ルールを満たし、まだ空いている頂点か
fn is_open_spot(board: &Board, n: NodeId) -> bool {
    let topo = Topology::get();
    if board.building[n as usize].is_some() {
        return false;
    }
    topo.node_neighbors[n as usize]
        .into_iter()
        .all(|m| board.building[m as usize].is_none())
}

/// 頂点 `node` に開拓地を建てた時の評価値。
///
/// 「自分が既に持っているもの」との相互作用込みで測るので、
/// 2 軒目は 1 軒目を補完する場所が自然に高く出る。
pub fn score_settlement(view: &View, node: NodeId, w: &PlacementWeights) -> f32 {
    let topo = Topology::get();
    let board = view.board();
    let me = view.me;

    let node_pips = board.node_pips(node);
    let mine = my_production(board, me);
    let held_numbers = my_numbers(board, me);
    let held_tiles = my_tiles(board, me);

    let mut score = 0.0f32;

    // --- 産出 ---
    let total_pips: u16 = node_pips.iter().map(|&v| v as u16).sum();
    score += w.pip * total_pips as f32;

    // --- 資源の多様性（まだ持っていない種類を増やす） ---
    for i in 0..NUM_RESOURCES {
        if node_pips[i] > 0 && mine[i] == 0 {
            score += w.variety_new;
        }
    }

    // --- 数字とヘクスの重複 ---
    for t in &topo.node_tiles[node as usize] {
        let num = board.tile_number[t as usize] as usize;
        if num == 0 {
            continue; // 砂漠
        }
        let pips = catan_core::board::pips(num as u8) as f32;
        if held_numbers[num] > 0 {
            score += w.number_dup * pips;
        }
        if held_tiles[t as usize] {
            score += w.same_hex;
        }
    }

    // --- 建設速度（この頂点を足した後の産出で測る） ---
    let combined: [u16; NUM_RESOURCES] =
        std::array::from_fn(|i| mine[i] + node_pips[i] as u16);
    let mut ports = board.ports_of(me);
    if let Some(p) = board.node_port[node as usize] {
        ports.push(p);
    }
    let eff = effective_rates(combined, &ports);
    score += w.etb_expand * speed(&COST_EXPAND, &eff);
    score += w.etb_city * speed(&COST_CITY, &eff);
    score += w.etb_dev * speed(&COST_DEV, &eff);

    // --- 港 ---
    if let Some(p) = board.node_port[node as usize] {
        match p {
            PortKind::Generic => score += w.port_generic,
            // 2:1 は「その資源をどれだけ産出しているか」で価値が決まる
            PortKind::Specific(r) => {
                score += w.port_specific * (combined[r.idx()] as f32 / 5.0).min(2.0)
            }
        }
    }

    // --- 拡張余地（2 歩で届く空き頂点） ---
    let mut room = 0;
    for m in &topo.node_neighbors[node as usize] {
        for k in &topo.node_neighbors[m as usize] {
            if k != node && is_open_spot(board, k) {
                room += 1;
            }
        }
    }
    score += w.expansion * room as f32;

    score
}

/// 初期配置で置く開拓地を選ぶ。
pub fn best_setup_settlement(view: &View, candidates: &[NodeId], w: &PlacementWeights) -> NodeId {
    let mut best = candidates[0];
    let mut best_score = f32::NEG_INFINITY;
    for &n in candidates {
        let s = score_settlement(view, n, w);
        if s > best_score {
            best_score = s;
            best = n;
        }
    }
    best
}

/// 初期配置の道を選ぶ。
///
/// 道そのものに価値は無いので、「2 歩先の一番良い空き頂点へ向かっているか」で選ぶ。
/// 公式アルマナックの Tactics も「囲まれることに注意」と書いている。
pub fn best_setup_road(view: &View, candidates: &[EdgeId], w: &PlacementWeights) -> EdgeId {
    let topo = Topology::get();
    let board = view.board();

    // 候補の辺すべてに共通する頂点 = いま置いた開拓地
    let near = {
        let [a, b] = topo.edge_nodes[candidates[0] as usize];
        if candidates
            .iter()
            .all(|&e| topo.edge_nodes[e as usize].contains(&a))
        {
            a
        } else {
            b
        }
    };

    let mut best = candidates[0];
    let mut best_score = f32::NEG_INFINITY;
    for &e in candidates {
        let [a, b] = topo.edge_nodes[e as usize];
        let far = if a == near { b } else { a };

        // far は自分の開拓地の隣なので距離ルールで建てられない。
        // その先（開拓地から 2 歩）が実際の狙い先になる。
        let mut reach = f32::NEG_INFINITY;
        for m in &topo.node_neighbors[far as usize] {
            if m == near || !is_open_spot(board, m) {
                continue;
            }
            reach = reach.max(score_settlement(view, m, w));
        }
        // 行き止まりでも「まだ枝がある方」を選ぶ
        let branches = topo.node_edges[far as usize].len() as f32;
        let s = if reach.is_finite() {
            reach + branches
        } else {
            branches - 1000.0
        };
        if s > best_score {
            best_score = s;
            best = e;
        }
    }
    best
}
