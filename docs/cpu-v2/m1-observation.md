# M1: 席別の観測イベントと履歴 API（実装済み）

## 何を作ったか

| ファイル | 役割 |
|---|---|
| `crates/catan-core/src/observation.rs` | 観測契約の型: `ObservationProfile`（`LEGACY_APP`）, `Observation`（公開状態＋本人の私的状態＋合法手）, `ObservedEvent`（席ごとに投影された出来事＋直前の公開文脈 `EventContext`＋直後の公開要点 `EventAftermath`） |
| `crates/catan-core/src/observer.rs` | 信頼境界。`redact()` が秘密を消した `Game` を作る（相手の資源・発展・今手番購入の内訳・山の内訳・本番 RNG を 0/定数に置換）。`observe()` と `project_event()` が席ごとの投影を作る |
| `crates/catan-core/src/view.rs` | `View::observe(legal, seq)` が v2 の**唯一の入口**。v2 は `View` の他のメソッドを呼ばない |
| `crates/catan-ai/src/bots.rs` | `Bot` に `wants_events()` / `observe(&ObservedEvent)` を追加（既定は false / 何もしない。旧ボットは無変更） |
| `crates/catan-ai/src/harness.rs` | `notify_bots()` / `notify_seats()`。全席分を**他人の手番中も**配る。旧ボットしか居なければ投影しない |
| `crates/catan-wasm/src/lib.rs` | `apply_notify()` で全 9 か所の適用を置換。**盗んだ資源は当事者以外に `"HIDDEN"`**（M0 で見つかった漏れの修正） |
| `crates/catan-server/src/rooms.rs` | `commit` / `step_bot` / `apply_maritime` の 3 か所で `notify_seats()` |
| `web/app.js` | `animSteal` が `"HIDDEN"` なら裏向きの札を飛ばす |
| `crates/catan-ai/src/history.rs` | 公開台帳: 購入世代（手番・枚数・購入時点の本人手番数）、使用、手番ごとの使用機会 `TurnRecord`、開いている交易の条件。**内部カード ID は持たない** |

## 出来事（`VisibleEvent`）

SetupSettlement{gained} / SetupRoad / Roll{d1,d2,produced[4]} / Discard{count,bundle:Option} / RobberMoved{tile,victim,stole_any,stolen:Option} / BuildRoad{free} / BuildSettlement / BuildCity / BuyDev{card:Option} / PlayKnight / PlayRoadBuilding / PlayYearOfPlenty / PlayMonopoly{res,taken[4]} / Maritime / Offer / Accept / Reject / CounterAlt / CounterAltRemove / Counter / Confirm{partner,give,want} / AcceptCounter{partner,alt,partner_gives,proposer_gives} / Cancel / EndTurn / Hidden

- `stolen` は奪った人・奪われた人（または公開プロファイル）だけ `Some`。第三者には `stole_any` だけ。
- `BuyDev.card` は本人だけ `Some`。
- `Discard.bundle` は `legacy_app` では公開（UI が内訳をログに出しているため）。
- `EventContext`（直前）: 盗賊の位置と各人の塞がれた pip、騎士数、最長路、道の残り、合法な道の有無、建てられる頂点数、銀行、枚数、公開点、`dev_played_this_turn`, `rolled`。相手モデルの特徴量はここからだけ作る。
- `EventAftermath`（直後）: 枚数・公開点・山の枚数・手番プレイヤー・勝者。推定の自己検査（粒子の枚数が観測と合うか）に使う。

## 順序番号と再接続

- `seq` は単調増加。`History::record` は `seq <= last_seq` の出来事を捨てる（重複配信の防止）。
- 出来事が届かなかった場合（順序が飛んだ・ホストが配っていない）は、判断時に `Belief::resync_if_needed(obs)` が観測スナップショットの枚数と粒子を照合し、食い違えば `resync()` で**観測だけから**粒子を作り直す（履歴は失われる。`diag.resyncs` に記録）。秘密には戻らない。

## テスト

- `crates/catan-ai/tests/fairness_v2.rs`
  - `秘密を差し替えても観測は一致する`: 相手の資源の分け方・発展の種類・本番 RNG を変えた双子で `Observation` が bit 単位で一致（12 seed × 約 40 局面）。相手席から見た公開部分も一致。
  - `秘密を差し替えても投影された出来事は一致する`: 双子に同じ手（偶然は固定）を適用し、第三者視点の `ObservedEvent` が一致。承諾者・提案者が「その札を持つ」公開情報を壊す双子は使わない。
  - `秘密を差し替えても判断と説明は一致する`: 同じ出来事列を流した 2 つの v2 が、秘密だけ違う局面で同じ手・同じ説明・同じ予測を返す（10 seed × 3 手ごと）。
  - `相手の伏せた勝利点はv2の判断を変えない`: 勝利点 2 枚 ↔ 騎士 2 枚の入れ替え（20 seed）。
- 動作確認: `node docs/cpu-audit-20260907/smoke_wasm.mjs`（再ビルドした `web/catan_wasm.wasm`）3 人戦 4 局＋4 人戦 4 局、全局決着、判断 中央値 0.10ms / p95 1.87ms / max 10.8ms（旧 CPU のまま。v2 は未搭載）。

## 未実装・限界

- オンラインの対不正性（サーバ権威方式）は範囲外。サーバは引き続き seed と全行動を配る。
- `counter_drafts_public = true`: 対案の候補の積み下ろしはエンジンの行動として全員に配られている（既存 UI の仕様）。
- 銀行は正確に公開（`bank_exact`）。非公開プロファイルは型だけ用意し、推定側の「銀行を潜在変数にする」処理は未実装。
