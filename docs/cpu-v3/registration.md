# v3 candidate freeze / final evaluation registration

Frozen before inspecting held-out results on 2026-09-10.
Candidate: CLI v3, V2Config::v3(): depth 4, max_nodes 60000, 3 belief samples;
new turn_planner.rs, existing v2 belief/hazard and setup evaluation.
Control: CLI v2, unchanged V2Config::default() (current shipped maximum).
Candidate selection: pilot 24 games seed 9100000; development 240 games seed 9200000.
Development: 76/240 wins, all 240 finished. No weight search on these games.

Held-out fixed N: 4-player 960 games, four 240-game blocks at seeds
9300000, 9300240, 9300480, 9300720. 3-player 480 games, four 120-game
blocks at seeds 9400000, 9400120, 9400240, 9400360.
All seat permutations rotate in each block; domestic trade on; no time limit;
400-turn / 100000-action cap; unfinished games stay in the denominator.

Adopt if the Wilson 95% interval is wholly above 25% for 4-player and above
33.333% for 3-player, zero unfinished games, new tactical/fairness tests and
workspace tests pass, and measured decision max is below the user's 30s budget.
Web WASM must also complete 3- and 4-player smoke games before delivery.
These results say nothing about a global optimum or human/Colonist win rates.
The prior produce_real test omitted required observation events; its host loop
is corrected independently, without changing the frozen candidate algorithm.
