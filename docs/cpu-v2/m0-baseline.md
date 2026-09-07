# M0: 現行版の凍結と台帳（2026-09-07）

## 凍結した対照 CPU

| 名前 | 中身 | 用途 |
|---|---|---|
| `v2_control` | `SearchBot::tuned_worlds(seed, 3, EvalWeights::tuned(), PlacementWeights::tuned())` = 設計書作成時の `max`（さいきょう）。2手読み・推定3通り | v2 との主対照。**途中で更新しない** |
| `baseline` | `SearchBot::with_weights(seed, 2, tuned, tuned)` = 2手読み・推定1通り | 副対照（弱い旧方式） |

`v2_control` は CLI 名だけを追加した別名で、生成されるボットは `max` と同一（同じ seed で同じ手を指すことをテスト `v2_controlはmaxと同一` で固定）。

## 作業ツリーの固定（HEAD だけでは足りない。未コミット変更あり）

- HEAD: `5ca9b70d54830ce88664628d5957f6ee6a23d673`
- `git write-tree`（追跡ファイル＋index）: `e5f56e38a3f17b798fc16896b814db7d53bcf1f7`
- 追跡＋未追跡（target/ 除く）の全ファイル sha256 を連結した sha256: `290ce0ad1b22dac7a5f2ad523ef37a2c80a55a0885a25a532a98e8704419175b`
  - 再計算: `(git ls-files; git ls-files --others --exclude-standard) | sort -u | grep -v '^target/' | xargs sha256sum | sha256sum`
- 未コミットの変更（凍結時点）: `README.md crates/catan-ai/src/{bots,search}.rs crates/catan-cli/src/main.rs crates/catan-core/src/view.rs crates/catan-server/src/rooms.rs crates/catan-wasm/src/lib.rs web/catan_wasm.wasm` ＋ 未追跡 `colonist-ui-notes.md crates/catan-ai/tests/fairness.rs docs/cpu-audit-20260907/ docs/cpu-optimal-*.md`

## バイナリ（凍結時点で `cargo build --release -p catan-cli` した直後）

| ファイル | sha256 |
|---|---|
| `target/release/catan.exe` | `c98109959b2832452dbcb4542e1423a27676d66116e3a0e1f6e3c26e94f53b1d` |
| `web/catan_wasm.wasm` | `e59a1db8142562d8118f49c290332f8c093454beeb4348d51a061ad71c00c217` |
| `target/release/catan-server.exe` | `3a1fa3ce899e26e145ef4d41c175a2929f2d21187ec1753441a85cc5c28c7969` |

ツールチェーン: rustc 1.96.0 (ac68faa20 2026-05-25) / cargo 1.96.0 / Windows 11 / 16 論理コア。

## ルールと観測のバージョン

- `rules_version = "base-5th-2020/v1"`: 基本セット 3〜4 人。10 点先取。手番の持ち時間は測定では無制限（`--no-time-limit`）。実験上限 400 手番は**ルール上の引き分けではない**（未決着として分母に残す）。
- `observation_profile = "legacy_app"`: [observation-contract.md](observation-contract.md) の表のとおり。

## 動作確認用ベンチマーク（強さの証明には使わない）

```powershell
Set-Location C:\catan_lab
.\target\release\catan.exe match max baseline baseline baseline --games 24 --seed 1700000 --no-time-limit
```

凍結時の出力: 24 戦・決着 24・平均 76.5 手番・7 ゲーム毎秒。さいきょう 7 勝（29.2%）。3.6 秒。
（24 局は再現性の確認用。区間は ±9pt 幅があり強さの根拠にならない）
