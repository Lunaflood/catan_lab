# colonist.io UI 実測メモ

自分のカタンの UI を作り直すための調査記録。2026-09-07。

方法は 2 本立て。
- **静的抽出**（遊ばずに取れるもの）: 音声 48 本・スプライト 376 コマ・SVG 63 個・
  文言カタログ 4696 件・CSS 実値。これが情報量では大半。
- **実測**（遊ばないと取れないもの）: 盤の tween アニメーション、当たり判定、手数。

環境: Chrome 1920x911 デスクトップ版（`containerLandscape`）／自作ルーム `pack7154`／
2 人対戦（赤・青とも自分で操作）／ターンタイマー 200 分。
⚠ 最初に内蔵ブラウザで見た `colonist.io/mobile` は縦持ち版で別物。不採用。

---

# 第 1 部 静的に取れたもの

## 1. 効果音 — 全 48 本

ファイル名が用途をそのまま表している。`AudioBufferSourceNode.start` を横取りして
実際の発火も確認済み（例: 盗賊確定と同時に `robber_place`、UI クリックで `click`）。
音声は 48kHz。

### 一覧（用途別）

| 分類 | ファイル |
|---|---|
| サイコロ | `dice_roll_1`〜`4`（**4 種をランダムに使い分け**） |
| 手番 | `your_turn` / `first_reminder` / `clock_tick` / `vote_reminder` |
| 建設 | `road_place` / `settlement_place` / `city_place` / `city_wall_place` / `city_destroy` / `city_improvement` / `metropolis_place` |
| 船（拡張） | `ship_place` / `ship_move` / `pirate_place` |
| 盗賊 | `robber_place` |
| 騎士 | `knight_equip` / `knight_place` / `knight_upgrade` |
| 発展カード | `devcard_bought` / `devcard_used` / **`devcard_monopoly`（独占だけ専用音）** |
| 捨て札 | `discard_broadcast` / `discard_notification` |
| 交易 | `offer_acceptable` / `offer_not_acceptable` / `offer_accepted` / `offer_rejected` |
| 実績 | `achievement_longest_road` / `achievement_largest_army` / `achievement_ck_progress` |
| 進行 | `game_started` / `game_started_guitar` / `settlement_phase_ended` / `settlement_phase_ended_guitar` / `victory` |
| 部屋 | `join_room` / `leave_room` / `room_setting_updated` / `room_get_ready` |
| 通信 | `reconnect` / `disconnect` |
| その他 | `click` / `message_notification` / `beginner_hint_activated` |

★ 設計上の要点
- **サイコロだけ 4 バリエーション**。同じ音の連発を避けている
- **交易は「受けられる/受けられない/成立/拒否」の 4 音**に分けている
- **捨て札は「自分が捨てる」と「他人に知らせる」で別音**
- **独占だけ専用音**。他の発展カードは共通の `devcard_used`
- 開始と初期配置終了に **通常版とギター版の 2 種**（設定で切替と思われる）

### 音の性質（波形を数値解析した結果）

`env` は 8 分割 RMS（ピーク比 0-9）、`band` は 125/250/500/1k/2k/4k/8kHz のエネルギー（0-9）。

| 音 | 長さ | 立上り | env | band | 読み取れる性格 |
|---|---|---|---|---|---|
| dice_roll_1 | 1008ms | 35ms | `11100000` | `1144970` | 2-4kHz 中心の硬い連打。前半 380ms だけ鳴って余韻なし＝**カラカラ** |
| road_place | 731ms | 36ms | `10000000` | `3793001` | 250Hz と 1kHz。頭だけ。**ゴトッ** |
| settlement_place | 731ms | 38ms | `10000000` | `6984110` | 500Hz 主体で低め。**ドスッ** |
| city_place | 731ms | 43ms | `21000000` | `6951200` | settlement より低域が厚く長い |
| robber_place | 967ms | 48ms | `11110000` | `9771211` | **125Hz が最大**＝重い足音系 |
| your_turn | 1944ms | 39ms | `21000000` | `2453591` | 4-8kHz が立つ**明るいチャイム**、約 2 秒 |
| victory | 2966ms | **425ms** | `12232110` | `9221000` | 低域主体・立上り遅い・約 3 秒＝**ファンファーレ** |
| achievement_longest_road | 2351ms | 344ms | `11121000` | `9200000` | 低域のみ。ゆっくり立ち上がる |
| devcard_monopoly | 2351ms | **634ms** | `01321100` | `9320000` | 溜めが長い。中盤で山が来る |
| offer_acceptable | 731ms | 42ms | `21100000` | `4594110` | 1kHz 中心の**ピコン** |
| offer_not_acceptable | 731ms | 40ms | `11100000` | `3912000` | 250Hz 寄り＝低い否定音 |
| click | **78ms** | 37ms | `00021000` | `1647931` | 4kHz の極短クリック |
| clock_tick | 288ms | 76ms | `01200000` | `0930311` | 250Hz の単発 |
| discard_notification | 1881ms | 50ms | `12100000` | `9912100` | 低域＋長い＝注意喚起 |

★ 全体傾向: **UI 音は 731ms 前後で統一**、**通知・実績・勝利は 2〜3 秒**、
**クリックだけ 78ms**。立ち上がりは 35〜50ms が標準で、
「溜めたい音」（勝利・独占・実績）だけ 300〜600ms かけている。

---

## 2. アニメーション — 資産としては存在しない

- スプライトシート 5 枚 376 コマを調べたが、**コマ送りの連番が無い**
  （連番は数字トークン `prob_2`〜`prob_12` の 10 個だけ）
- 使用ライブラリは **PixiJS**。**GSAP は入っていない**
- つまり**盤の動きは全部コード内の手書き tween**。難読化されていて定数も読めない
- → 盤側のアニメは**連射スクリーンショットで実測するしかない**（第 2 部に記載）

**DOM 側だけは `transition` の実値が取れる:**

| 対象 | 値 |
|---|---|
| ボタン押下 | `transform 0.05s ease-in-out`（**50ms**。かなり速い） |
| 手札にカードが入る | `slideInHorizontal 0.12s linear forwards` |
| サイコロの出入り | `transform 0.3s, opacity 0.3s` |

---

## 3. スプライト資産の内訳（376 コマ）

| 接頭辞 | 数 | 内容 |
|---|---|---|
| `knight_` | 85 | 騎士（レベル 1-3 × 色 × 稼働/非稼働 × 移動） |
| `icon_` | 54 | UI アイコン |
| `ship_` | 51 | 船（方位別） |
| `card_` | 46 | カード面（資源 5・発展カード各種・裏面） |
| `city_` | 36 | 都市（色別＋`destroyed_` 破壊版） |
| `tile_` | 29 | 地形（`_empty` 付きの変種あり・`fog`・`gold`） |
| `road_` | 13 | 道（色別＋`highlight`） |
| `settlement_` | 12 | 開拓地（色別） |
| `player_` | 12 | プレイヤー背景色 |
| `button_` | 12 | ボタンのバッジ背景（色別） |
| `prob_` | 10 | 数字トークン 2〜12 |
| `port_` | 8 | 港（`pier` 桟橋・`highlight`・資源別） |

★ 色は white/black/blue/red/green/orange/bronze/gold/silver/pink/mysticblue まで用意。
★ **駒は色ごとに別画像**（tint ではない）。UI 側は SVG 63 個（`road_blue.svg` など）。
★ `tile_*_empty` があるのは、**数字トークンを外した状態**を別絵で持っているため。

---

## 4. 文言カタログ（4696 件）

`ja_strings.json` / `en_strings.json` に、ゲームが出しうる文言が全部入っている。

### 4-1. 状態表示は必ず「自分版」と「他人版」の対

```
turn.diceSelf        Roll Dice
turn.diceOther       {{playerUsername}} is rolling dice
turn.turnSelf        Your Turn
turn.turnOther       {{playerUsername}}'s Turn
turn.tradingSelf     Answer Trade
turn.tradingOther    {{playerUsername}} Answering Trade
turn.tradingOther_other  Players Answering Trade   ← 複数形も別に持つ
turn.waitingForOthers    Waiting for others...
```

★ **すべての状態に Self / Other を用意する**のが colonist の設計。
待っている側の画面が絶対に無言にならない。複数形の別文言まである。

### 4-2. 発展カード（全 5 種）— 各カードが 3 層のテキストを持つ

| カード | title | body（ホバー） | popupBody（確認） |
|---|---|---|---|
| 騎士 | Knight | Place the Robber on any tile and steal 1 card from another player. | Place the Robber anywhere on the map and steal a random card from a player with a settlement or city on that tile |
| 街道建設 | Road Building | Place 2 roads for free. | Place 2 free roads. |
| 独占 | Monopoly | Steal all cards of a single type of resource from every player. | — |
| 収穫 | Year of Plenty | Select any 2 resources from the bank. | — |
| 勝利点 | Point | Secretly gives 1 point. | **Secretly awards 1 point; it is automatically used.** |
| 裏面 | Development Card | Can be purchased for Wool, Grain, and Ore. | — |

★ **勝利点カードは自動使用**。プレイ操作が無い。
★ 資源カードにも同じ 3 層があり、「その資源が何に使えるか」を書いている
（例: Lumber = "Can be used to build roads & settlements."）。**拡張ルールごとに文言を差し替える**。

### 4-3. 誘導文（prompts）— 初期配置には見出し＋補足がある

```
initialPlacements.firstSettlement.title  Place your first settlement
initialPlacements.firstSettlement.body   Get resource cards from surrounding tiles when the dice rolls their number.
initialPlacements.secondSettlement.body  You will receive resources from each of the surrounding tiles as your starting hand.
initialPlacements.firstRoad.title        Place a road
initialPlacements.firstRoad.body         All settlements must be a minimum of two roads apart.
initialPlacements.secondRoad.body        Tip: Head to a port! Settlements on ports reduce the cost of trading with the bank.
playerRolledSeven   7 rolled! Move the Robber to a tile to block its production. Choose a player on that tile to steal from.
selectWhoToRobFrom  Choose a player to steal 1 random card
discardCards        Discard Cards
youHaveMoreThanXCards  7 was rolled! You have more than {{count}} cards. You need to discard {{amountToDiscard}} of them.
```

★ 実際の対局では簡潔な状態バーしか出ないが、**初心者モード用に「見出し＋なぜそうするのか」の
2 行案内が全状況分そろっている**（`beginnerMode` 22 件、`sfx_beginner_hint_activated` も存在）。

### 4-4. 港の説明（6 種）

```
general  General Port  3 of the same resource → any other resource
lumber   Lumber Port   2 Lumber → any other resource
brick / wool / grain / ore  同上（2:1）
```

### 4-5. エラー文言（48 件）— 交易だけで 16 件以上

```
canOnlyInitiateTradeFromYourTurn     Not your turn
doesNotHaveOfferedCards              You do not have the offered cards
tradingSameTypeCards                 Same type of card
identicalTradeLimit                  Identical trade limit
tooManyActiveTrades                  Too many active trades
tooManyWantedCards                   Too many wanted cards
cannotExecuteOpenEndedTrade          Cannot accept open ended trade
only1HumanPlayer                     No player to trade with
noOneHasThatResource                 No one has wanted resource
playerBlockedTradingWithYou          {{playerUsername}} blocked trading with you
everyoneBlockedTradingWithYou        Everyone blocked trading with you
youBlockedTradingWithAllPlayers      You blocked trading with all players
cannotTradeDevCard                   A Development Card cannot be traded
（実測で見たもの）開発カードを購入したターンにはプレイできません
```

★ **「プレイヤーごとに交易を拒否できる」社交機能がある**（block）。
★ エラーは全部**短い一文**。理由だけ言って、やり直し方は言わない。

---

# 第 2 部 遊ばないと取れなかったもの

## 5. 画面レイアウト（実寸 px / 1920x911）

```
┌─ 左レール 62 ─┬──────── 盤 canvas 1590x911 ────────┬─ 右 361 ─┐
│ ⚙ 📖 ⛶ ⓘ     │  UI は canvas の上に重ねる          │ ログ      │
│ 43x43 / 56間隔│  （盤の描画域を削らない）           │ チャット  │
│              │                                    │ 銀行      │
│              │         🎲🎲 出目は出しっぱなし     │ 相手カード│
│              │                                    │ 自分(大)  │
└──────────────┴────────────────────────────────────┴───────────┘
        [手札 678x85]        [状態バー][残り時間]  ← ボタンの真上
                             [交易][発展][道][開拓][都市][⏩]  85x85 x6 / 90px間隔
        画面最下端に 4px の時間バー（全幅）
```

- 残り駒数（道 13 / 開拓地 3 / 都市 4）は**ボタンの上にバッジ**
- **タブのタイトルが状態で変わる**:
  自分の番＝「アクションが必要です!」／待ち＝「プレイヤーの皆さんお待ちしています！」

## 6. 配色・タイポグラフィ（computed style 実値）

| 用途 | 値 |
|---|---|
| フォント | `"Open Sans", sans-serif` 基準 16px、UI は 1.113 倍（実効 17.8px） |
| 背景（海） | `#328DC1` |
| 濃色地の文字 | `#F6F8FA` |
| 明るいパネル（自分・ログ・銀行） | `linear-gradient(#FCFAF5, #E2D7C4)` 角丸 4px |
| **相手のカード（非アクティブ）** | `linear-gradient(#ADADAA, #A59D90)` ← 灰色に沈める |
| 明るいパネル上の文字 | `#000000` |
| 数量バッジ | `#285FBD` / 22x22 / 角丸 `2px 2px 2px 22px`（左下だけ大きく丸いカード形） |
| ログの区切り | `<hr>` 2px `#808080` |
| 駒・アバターの色 | 赤 `#EC6F66→#EC6961` / 青 `#52B1CC→#4FABCB`（放射グラデ） |
| **明るい地の名前の色** | 赤 `#CF4449`（駒より暗い版）・`font-weight:600` |

サイズ: 自分のカード 338x125（アバター 71px 丸）／相手 338x80（40px 丸）／
ログのアバターと資源アイコン 22x22（**道だけ 5x22**）／資源カード 45x62。

★ **駒の色と、白地に載せる文字色を別に持っている**。自分のゲームで `PLAYER_INK` を
作った判断と同じ結論に colonist も達している。

## 7. 操作モデル

### 盤は「置ける所が全部光る → 押す → 真上の確定バッジを押す」の 2 クリック

- 種類をメニューから選ぶ段階が無い（初期配置・盗賊）
- 別の場所を押すと選択が移るだけ。キャンセル操作が要らない
- Enter では確定できない

当たり判定（hex 半径 R=51.3px のとき）:

| 対象 | マーカー中心 | 確定バッジ中心 |
|---|---|---|
| 頂点（開拓地・都市） | 頂点の 15px 上 | マーカーの 47px 上 |
| 辺（道） | 中点の 15px 上 | マーカーの 47px 上 |
| マス（盗賊） | 中心の 35px 上 | マーカーの 43px 上 |

⚠ 縦に並ぶ頂点の間隔は 54px。下側の頂点を選ぶと確定バッジが上の頂点のマーカーと
7px しか離れず**誤爆する**。colonist にもこの欠陥がある。真似するなら要対策。

### 本編の建設は逆で「ボタン → 盤」

### 発展カードは 3 段階

1. ホバー → カードが持ち上がり、上に黒いツールチップ（title + body）
2. クリック → 手札の上に**効果文＋[✗][✓] の確認ストリップ**
3. ✓ → 発動

### ルール違反は左下の一瞬のトースト。モーダルは出さない

### 通信

1 クリック = 37 バイトのフレーム 1 本（`action`/`payload`/`sequence`）。
**選択の時点でもサーバに送っている**＝ 1 手 2 往復。
⚠ `WebSocket.prototype.send` を差し替えると数分で切断される（改ざん検知の疑い）。

## 8. ログの作り

```
[アバター 22x22] [名前=その人の色/weight 600] が [22x22 アイコン] を配置しました
```

- 本文は黒、名前だけプレイヤー色（明るい地用の暗い版）
- 資源・駒はテキストでなく画像を文中に埋め込む（道だけ 5x22）
- **ターンの切れ目に `<hr>`（2px #808080）**
- **自分の行動は二人称**（"You stole 🟩 from jinsan"）、他人は三人称
- 記録が広い: 交易の**意思**（成立前）／盗賊の封鎖／得点の増分 `(+1 VP)`
- ⚠ 仮想スクロールなので、DOM 監視すると同じ行が再追加される

## 9. 交易 UI

### 作る（下から 734x322 がせり上がる。盤に重なる）

```
① 欲しいカードの見本  🌲 🧱 🐑 🌾 🪨 ❓          [レート]
② もらう  👥 ↓  [積んだカード]
③ わたす  🔴 ↑  [積んだカード]
④ 自分の手札          各カードの下に「4:1」など**銀行レートを直接表示**
```

- ①を押す＝もらう +1／④を押す＝わたす +1。**入力欄もステッパーも無い**
- 確定は右に 2 つ: 🏛 銀行と交易 / 👥 プレイヤーと交易
- 交易ボタン自身が ✗（閉じる）に変わる

### 受ける（盤の右上に重なるカード・折りたたみ可）

```
[提案者アイコン]                          ^
 🟠↓ [🧱x1]              [他家 ✗][他家 ✓]   ← 他の人の回答が見える
 🔴↑ [🌲x1]              [✏️対案][✗断る][✓受ける]
```

- 3 択が 1 行。**払えない提案は ✓ が無効化**
- 新しい提案は同じ枠を置き換える（積み上がらない）

## 10. アニメーション（連射で実測）

### サイコロ
DOM 要素（89x89）。`transition: transform 0.3s, opacity 0.3s` で出入りし、
**出目そのものを押して振る**。音は `dice_roll_1〜4` からランダム（1008ms）。

### 資源獲得
1. 出目確定 → **産出マスが暗く沈む**
2. **そのマスの上に資源カードが生成**
3. **曲線を描いて手札へ飛ぶ**（複数マスは同時に別経路）
4. 着弾して枚数が更新（手札側は `slideInHorizontal 0.12s`）

### 発展カード購入（**双方向・交差する**）
1. 銀行帯（右上）の紫カードが持ち上がって光る
2. 紫カードが銀行から降りてくる
3. 同時に支払い 3 枚が手札から銀行へ飛ぶ
4. 2 つの流れが画面中央で交差する
→ 銀行を右上に置くことで**飛行距離を稼ぎ、何を払って何を得たかを読ませている**

### 盗賊移動 → 略奪
1. **置けるマス全部に丸い的**
2. マスを選ぶと**そのマスの数字トークンが持ち上がる**
3. 確定バッジ（盗賊の絵）
4. **盗賊が盤上を歩いて移動**（瞬間移動しない・複数フレーム）
5. **盗まれたカードが被害者のプレイヤーカードから飛び出す**（一瞬表向きになる）
6. 自分の手札へ降りてくる

### 使用済み発展カード
使った騎士は**銀行帯の発展カード枠に絵として表示**される。

## 11. 待機所（ルーム作成）

```
┌ プレイヤー (1/4) ┬────── 部屋ID: pack7154 [✕] ──────┬ チャット ┐
│ 自分 [✏️名前]    │ 友達を招待 [URL] [コピー]         │          │
│ カルマ/色/準備   │ ゲームモード（アイコン格子・🔒）   │ 入力欄   │
│ [ボットの追加]×3 │ 地図（アイコン格子）              │          │
│ 友達 (0/2)       │ ルール（トグルカード 5 種）        │          │
│                  │ 詳細設定                          │          │
│                  │  ターンタイマー  < 15/30/60/120/200 > │      │
│                  │  最大プレイヤー数 < 2〜8 >        │          │
│                  │  勝つためのポイント ──●── 3..20   │          │
│                  │  カード廃棄制限   ──●── 5..20     │          │
└──────────────────┴ [ゲームをスタート] ───────────────┴──────────┘
```

- **空席 1 つにつき「ボットの追加」ボタンが 1 個**（人数と CPU 追加が同じ UI）
- 招待は URL + コピーボタン
- ルール: プライベートゲーム / 銀行カードを隠す / フレンドリーな強盗 /
  バランスのとれたサイコロ / 荒らし禁止（有料）
- 数値は「< 値 >」ステッパー、範囲つきはスライダーと使い分け
- 参加者側に **「I'm Ready」チェックボックス**＋「全員が準備完了しないとホストは開始できません」
- 部屋一覧は **モード / 地図 / ターンタイマー / 席の埋まり具合** の 4 列
- 「銀行カードを隠す」ON でも**カードの厚み（重なり）は残る**ので残量は視覚的に分かる

## 12. 通信断の扱い

ログに「jinsan が切断されました。再接続しない限り、次のターンはボットが引き継ぎます」
「あなたが最後の 1 人です。相手が再接続しなければ 200 秒後に勝利が与えられます」と出る。
**ボット引き継ぎ + カウントダウン付き不戦勝**の 2 段構え。`sfx_disconnect` / `sfx_reconnect` あり。

---

# 第 3 部 自分のゲームへの適用案（優先度順）

1. **置ける場所を盤に直接光らせる**。メニューから種類を選ぶ段階を減らす
2. **状態バーを 1 か所に集約**。「やること」「残り時間」「相手が今やっていること」を全部そこに
3. **すべての状態文言を Self / Other の対で用意する**（待つ側を無言にしない）
4. **建設ボタンのホバーで必要資源を状態バーに出す**／払えないボタンは無効化
5. **残り駒数をボタン上のバッジに**
6. **ログの資源をアイコン画像に**し、ターン区切りに罫線。自分の行動は二人称
7. **交易は「見本を押す＝もらう／手札を押す＝わたす」の 1 クリック加算**
8. **手札に銀行レート（4:1 / 3:1 / 2:1）を直接表示**
9. **提案カードに他プレイヤーの回答を出す**
10. **エラーは左下トースト**。モーダルで止めない
11. 相手カードは灰色グラデ、自分だけ明るく大きく
12. **効果音は 731ms 前後で統一、クリックだけ 78ms、勝利/実績だけ 2〜3 秒**。
    サイコロは複数バリエーションを用意して連発を避ける
13. ⚠ 確定バッジを真上に浮かせる方式は縦並びの頂点と衝突する。要対策

---

# 第 4 部 追加実測（捨て札・本編の建設・銀行交易）

## 13. 捨て札 UI（7 が出て手札が上限超のとき）

上限は部屋設定「カード廃棄制限」（既定 7）。**8 枚以上で発動、切り捨てで半分を捨てる**。

### 上限超えの予告は 3 か所に出る

1. **手札バーの地色がピンク〜赤に染まる**
2. **プレイヤーカードの手札枚数に赤い「!」**が付く（自分・相手とも）
3. 捨て札が始まると下記のパネル

### パネルの構造（実寸）

```
actionBox [137,416 507x211]   ← 左下からせり上がり、盤に重なる
├ header [137,416 507x91]
│   ├ headerImage 40x69   icon_discard_resource_cards（カードに赤い上向き矢印）
│   └ タイトル「カードを捨てる (0/6)」  ← 赤字・太字。()内は ライブ counter
│      本文「7が出ました！7枚以上のカードを持っています。◯枚のうち 6 枚を
│           捨てる必要があります。」  ← 黒・小さめ
├ interactiveBody [137,507 507x65]   ← 選んだカードが並ぶ帯
└ footer [137,572 507x55]  右端に ✓（枚数が揃うまで無効）
      ↓
[手札バー]  ← ここを 1 回押すと 1 枚が上の帯へ移動
```

★ 要点
- **手札のカードを 1 回押す＝1 枚選択**。交易パネルと同じ「クリック回数＝枚数」方式
- **counter は必要数に達すると赤→緑に変わり、同時に ✓ が有効化**される
- 確定すると**カードが手札から銀行へ飛ぶ**。相手の画面からも飛んでいるのが見える
- 音は **`discard_notification`**（1881ms・低域主体の長め）。
  他人に知らせる `discard_broadcast` は別ファイル
- ログは「jinsan 破棄されました 🟩🟩🟩🟩🟨🟨」＝**捨てた内容がカード絵で全員に公開される**
- 状態バーは `Select Cards to Discard` ／ 他人待ちは **「他のプレイヤーが捨てる」**（Self/Other 対）

## 14. 本編の建設（初期配置と操作が違う）

**初期配置**: 盤を押す → 確定バッジ → 確定（2 クリック）
**本編**: 下のボタンを押す → 盤を 1 回押すだけで確定（**確定バッジは出ない**）

- ボタンを押すと**状態バーが「道路を建設する」に変わり、その左に必要資源のカードが並ぶ**
- 置ける辺／頂点が盤上で光る
- **1 本置いてもモードは維持される**（続けて置ける）。別の場所を押すか他ボタンで抜ける
- ログの語が変わる: 初期配置=「道を**配置**しました」／本編=「道を**構築**しました」
- 音は `road_place`（実測で発火確認）
- ボタン上の残り駒数バッジが減る（13→12→…）

## 15. 銀行交易と港（4:1 の実測）

- 交易パネルを開くと、**自分の手札 1 枚ずつの下に交換レートが直接書かれる**。
  港を持っていなければ全部 **`4:1`**。港を持てば該当資源が `2:1`、一般港なら `3:1` になる
- 確定ボタンは右に 2 つだけ: **🏛 銀行と交易** / **👥 プレイヤーと交易**
- 港は**海に浮かぶ船の絵＋桟橋 2 本**で表現され、**桟橋が伸びた先の 2 頂点**が港の権利を持つ。
  スプライトは `port_pier` / `port_highlight` / `port_brick` など 8 種
- 文言（`game.ports`）: 一般港「同じ資源 3 枚 → 任意の 1 枚」／資源港「その資源 2 枚 → 任意の 1 枚」

⚠ **港を実際に使うところまでは到達できなかった。** 赤はレンガの産出地を持たず、
港の頂点まで道 3 本を使った時点で資源が尽き、その後 7 が続いて 2 回捨て札になったため。
UI 自体は銀行交易と同一（レート表記が変わるだけ）。

## 16. プレイヤー間交易の成立手順（2 段階だった）

1. 提案側が組んで「👥 プレイヤーと交易」
2. 受け手の画面に提案カードが出る → **[✏️ 対案][✗ 断る][✓ 受ける]**
3. 受け手が ✓ を押すと、**提案側に確認カードが戻る**（このカードは **[✓][✗] の 2 択のみ**）
4. 提案側が ✓ で成立

★ つまり**受諾は「予約」で、最終決定権は提案側にある**（複数人が受けた場合に選べる設計）。
状態バーは提案側 `Players Answering Trade`（日本語は「トレードに反応する選手たち」…誤訳）。

## 17. その他

- **タブのタイトルが状態で変わる**:
  自分の番＝「アクションが必要です!」／待ち＝「プレイヤーの皆さんお待ちしています！」
- **通信断**: 「切断されました。再接続しない限り、次のターンはボットが引き継ぎます」
  「あなたが最後の 1 人です。相手が再接続しなければ 200 秒後に勝利が与えられます」
  ＝ボット代打ち＋カウントダウン付き不戦勝の 2 段構え
- 音の発火は実測で確認済み: `click`（全 UI クリック）/ `dice_roll_*` / `your_turn` /
  `road_place` / `robber_place` / `discard_notification`

---

# 未確認（残り）

- **港を使った交易**の実操作（UI 構造とレート表示は把握済み・上記 15）
- **開拓地／都市を本編で建てる**ときのアニメーション（道は実測済み）
- **最長交易路・最大騎士力**の獲得演出（音 `achievement_longest_road` / `_largest_army` は解析済み）
- 発展カード 5 種のうち、**街道建設・独占・収穫・勝利点**の実操作
  （騎士は実測済み。文言・音・効果は 4 種とも取得済み）
- リザルト画面（対象外）

部屋 `pack7154` はタイマー 200 分で生存中。青の盗賊配置待ちで中断している。
