# v4 final evaluation registration — 2026-09-10

Candidate chosen before inspecting held-out results: CLI v4, V2Config::v4().
Includes depth-4 turn planning, 60000 expansion allocation, 3 belief samples,
3-model setup forecast, economic/army/race evaluation, structural hazard delta.
The decision-local reach cache preserves the mathematical input/output; its
signature includes every immutable solver parameter, hand, and node budget.
No weights or candidate policy may change after this registration.

Development (96 games each, seed 9700000):
- Full v4: 48 wins, 96 finished, versus v3 x 3.
- Setup only: 33 wins; strategy only: 43 wins. All finished.
- Earlier 24-game pilot preceded structural hazard delta and is not pooled.

Fixed held-out samples:
- v4 vs original shipped v2 x 3: 480 games, blocks of 120 at
  9800000 / 9800120 / 9800240 / 9800360.
- v4 vs v2 x 2: 240 games, blocks of 60 at
  9900000 / 9900060 / 9900120 / 9900180.
- Independent comparison vs the stronger intermediate v3 x 3:
  120 games, seed 9950000.
All blocks rotate complete seat permutations; trade on; no turn clock limit;
400 turns / 100000 actions cap; unfinished games remain in the denominator.

Adopt if Wilson 95% lower bound exceeds 25% for both 4-player comparisons and
33.333% for 3-player, zero unfinished games, tactical/fairness/workspace tests
pass, native measured max stays below 30 seconds, and 3/4-player WebAssembly
smoke games finish without an illegal move or stall.
Record block p95 maximum and measured maximum, not a pooled p95 estimate.
No claim about human/Colonist performance or globally optimal play.
