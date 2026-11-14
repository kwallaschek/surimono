# Status

Updated after every task. Times are JST.

> **Scheduled sessions:** if `scripts/fleet/owner-check.sh` prints STOP, or the heartbeat
> is under 3 hours old and names another session: change nothing more, end the session.

> **Kay, 5 Nov 09:07: no releases until phase 3 is done.** Drop the v0.1.0 re-cut
> (release/v0.1.0-fix and its review); don't touch tags; nothing goes to RubyGems. The engine
> time-limit fixes (#125, #126) merge into main as normal work. Kay copied the docs/ci workflows
> (9c4124bd); macOS now runs on macos-15. See DECISIONS 2026-11-05.

## Now (Sun 15 Nov 2026, 04:15)

Session cse_011c8hfpYaknCZDyxDjnJSQG holds the fleet (Kay, 11 Nov: full authority; directive: it
works, prints on par with Chrome-based tools, faster, less memory). Agents run on Fable 5.2 since
14 Nov 20:28 (Opus 5.5 from 13 Nov 22:22).

- **Merged 13 Nov 22:35 to 14 Nov 04:20:** #181 (selector cut that corrupted printed sheets),
  #183 (Raikiri measured as a printer), #179 (layout prototype, hidden), #180 (printer-safe output:
  4 of 6 real-world pages pass on stock fulgur, 2 without), #185 (Raikiri spike), #184 (setters
  that silently did nothing), #186 (docs). The merged trees of #179 and #180 were tested locally
  (clippy and the crate's tests) before pushing.
- **Decided 14 Nov (DECISIONS):** phase 3 layout queries target Raikiri behind the seam, reading
  its paged fragments (E3-R1 to E3-R4); no Blitz-specific layout work; the print path is designed
  around fulgur-raikiri; real layout stays internal until one layout is bounded in time (E3-01e).
- **Merged 14 Nov:** #187 Q-CF-02 (page-path benchmark: faster on 8 of 8 pages, 3.1 to 4.5x less
  memory), #188 P2-CF-07 (checkpoint-3 evidence regenerated with printer-safe output, 06:20).
- **Main is green** at e29e22f0 on every full.yml job (with #187 and #188).
- **Nothing ran from about 07:00 to 20:28 on 14 Nov:** the Opus limit stopped every agent. Kay
  switched the session to Fable 5.2 at 20:28; agents restarted on Fable from the pushed branches.
- **Merged 14 Nov evening:** #189 E3-R1 stage A (21:10: the `raikiri` feature, ICU 2.2 to 2.4,
  cargo-deny reads every feature; full.yml on that merge was green on every job, ASan included) and #190 P2-DOM-13 (21:45, pushed 22:10: the parsed `xmlns` prefix; katex's output now equals
  Chrome's DOM). No PR open in review.
- **Merged 15 Nov:** #191 P2-PKG-09 (01:30: CI lints and tests the `raikiri` feature) and #193
  (04:15: checkpoint-3 evidence regenerated on main; COMPAT gives each WPT target a status on both
  counts).
- **Open PR, blocked in review, author fixing (since 03:55):** #192 E3-R1 stage B (the Raikiri
  engine, hidden). Five blocking findings, all wrong numbers given silently: forced breaks on
  pseudo-elements, `float: footnote` and `position: running()` in the measuring cascade, boxes
  above the page top cut or lost (off-screen text measuring), no refusal past 10,000,000 px, and
  offset rounding of negative halves. The review is PR comment 6882493399; after the fixes a fresh
  reviewer takes the new head. Its DECISIONS lines are in an orchestrator comment on the PR, plus
  two more: the CSSOM View §7 conflict for offsets from `body`, and the page area as viewport when
  an author `@page` sets a size or margin.
- **Private, for Kay:** one more upstream finding from #192's review, in the Claude project
  (`claude/private-upstream-findings-2026-11-15.md`) and `.private-tmp/review-192.md`.
- **Kay, 15 Nov 04:27: no interactive capabilities** (DECISIONS). The dom/events target is judged
  without input-driven tests, so all four phase-2 WPT targets are met; P2-CF-17 puts that into
  COMPAT and the README.
- At most two agents at a time (DECISIONS 15 Nov): the usage limit stopped the fleet three times
  in a day.
- Clean /home/claude/target only while holding the build lock (`flock` on
  `/home/claude/target/.fleet-build-lock`): a cleanup at 00:35 broke a running gate.
- Finished worktrees were removed on 14 Nov (disk was full: 240 MB free, now about 6 GB); the
  branches stay on GitHub.
- Kay's queue: report the private findings (four files in the Claude project; the 13 Nov one has
  the slow fulgur pages and Raikiri's numbers on them); turn the scheduled resume task back on
  (and set it to Opus 5.5) if he wants night work.
- Standing permissions from Kay (11 Nov): merges on green CI at the exact head plus a fresh
  review; third-party printers on the fleet machine; history rewrite to his GitHub noreply
  address at a time the orchestrator picks (Q-ORC-03, when no PR is open); branches stay.

Next session, first: `scripts/fleet/ci-status.sh $(git rev-parse origin/main)` and wait for a
complete green full.yml on main. Then: the review of #192 (E3-R1 stage B); merge; then
Q-ORC-03 (history rewrite) in the first moment with no open PR; then by value: E3-R1, Q-PKG-04
(Japanese spacing for Raikiri), Q-PKG-03 (print media), P2-CF-09 (Chrome-equivalence check in
CI), P2-PKG-05, P2-DOM-11, P2-DOM-10, Q-PKG-05, Q-CF-03 part 2, Q-GFX-04, P2-CF-08, P2-CF-11,
P2-CF-12, P2-RT-11, P2-PKG-06, P2-PKG-07, P2-ORC-03. Push docs to main once per round, not per
merge. Checkpoint-3 evidence by Sun 15 Nov; digest Sun 15 Nov 20:00 JST; checkpoint 3 Sun 22 Nov.

## Earlier (Fri 13 Nov 2026, 22:35)

Session cse_011c8hfpYaknCZDyxDjnJSQG holds the fleet (Kay, 11 Nov: full authority; directive: it
works, prints on par with Chrome-based tools, faster, less memory). **Agents run on Opus 5.5 from
13 Nov 22:22** (Kay switched the session after the Fable limit). Nothing ran from 12 Nov ~15:30 to
13 Nov 22:22: every Fable agent stopped on the limit and the scheduled resume task is paused.

- **Main is green** at c8d13990 (every full.yml job).
- **Merged 14 Nov ~00:15:** #181 P2-DOM-08 (the selector cut that corrupted printed sheets) and
  #183 Q-CF-03 part 1 (Raikiri measured). #180 is blocked again on two new classes (author agent fixing).
- (22:35) Open PRs, all green on CI, all in fresh review (Opus reviewers started 22:30):
  #179 E3-00 at 37da987c (second round), #180 Q-PKG-02 at 7e363895 (second round, the
  187-page Chrome harness again), #181 P2-DOM-08 at d8efcdd2, #183 Q-CF-03 part 1 at dc2bc53e.
- **Running:** the three reviewers (one covers #181 and #183) and the DOM agent finishing
  P2-DOM-09 from WIP 5b6f202d (saved by the orchestrator from its worktree).
- Not restarted yet: Q-CF-03 part 2 (ironpress, rustyprint, forme-ruby).
- Standing permissions from Kay (11 Nov): merges on green CI at the exact head plus a fresh
  review; third-party printers on the fleet machine; history rewrite to his GitHub noreply
  address at a time the orchestrator picks (Q-ORC-03, when no PR is open); branches stay.
- Kay's queue: report the private findings upstream (three files in the Claude project) and
  file the docs/upstream drafts; turn the scheduled resume task back on (and set it to Opus 5.5)
  if he wants night work.

Next session, first: `scripts/fleet/ci-status.sh $(git rev-parse origin/main)`. A new session
can't resume this session's agents: continue each task from its pushed branch. Then: merge what
the reviews approve (local merge commits by the fleet identity; squash where a branch's history
must not reach main); hand blockers to new author agents. Then by value: E3-0R, Q-PKG-04,
Q-PKG-03, P2-CF-09, P2-PKG-05, Q-CF-03 part 2, E3-01c, Q-GFX-04, Q-GFX-03, Q-CF-02, P2-CF-07,
P2-CF-08, P2-CF-11, P2-RT-11, P2-DX-05, P2-ORC-03, and Q-ORC-03 when no PR is open.
Digest Sun 15 Nov 20:00 JST.

## Earlier (Thu 12 Nov 2026, 15:20)

Session cse_011c8hfpYaknCZDyxDjnJSQG holds the fleet (Kay, 11 Nov: full authority; directive: it
works, prints on par with Chrome-based tools, faster, less memory). Every agent runs on Fable 5.2.
The usage limit has stopped the agents six times; the scheduled resume task is paused (by its
owner), so work restarts only when Kay writes. Agents push early.

- **Main is green** at 4cf95afb (every full.yml job, nightly-extra included, after #182).
- **Raikiri result (Q-CF-03 part 1, #183):** fulgur's next layout engine passes 5 of 6
  real-world pages (stock fulgur: 2) and 8 of 13 gap reproducers (2), fails the five Japanese
  island pages on Japanese-Latin spacing, and costs about 5 ms and 7 MB more per page at the
  median. It has no Parley. Printer decision in DECISIONS (12 Nov): no Blitz-side patches,
  Raikiri tracked and spiked as a layout engine (E3-0R).
- **Open PRs:** #179 E3-00 (re-review running, head 37da987c), #181 P2-DOM-08 (review running,
  head d8efcdd2), #180 Q-PKG-02 (author fixing the review's five blockers; pushed 93f2d750 so
  far), #183 Q-CF-03 part 1 (head dc2bc53e, needs a fresh review).
- Running: reviewers of #179 and #181; PKG agent (#180); CF agent (Q-CF-03 part 2: ironpress,
  rustyprint, forme-ruby); DOM agent (P2-DOM-09: `el.hidden`, `document.body =`, an audit of
  setters that silently do nothing).
- Standing permissions from Kay (11 Nov): merges on green CI at the exact head plus a fresh
  review; third-party printers on the fleet machine; history rewrite to his GitHub noreply
  address at a time the orchestrator picks (Q-ORC-03, when no PR is open); branches stay.
- Kay's queue: report the private findings upstream (three files in the Claude project) and
  file the docs/upstream drafts; turn the scheduled resume task back on if he wants night work.

Next session, first: `scripts/fleet/ci-status.sh $(git rev-parse origin/main)`. A new session
can't resume this session's agents: continue each task from its pushed branch or its worktree
under /home/claude/wt/. Then: collect or redo the reviews of #179, #181, #180 (after its
author's push) and #183; merge on green plus approval as local merge commits by the fleet
identity; tool-touching PRs also need their tool tests run from a clean checkout. Then by value:
E3-0R (Raikiri behind the seam), Q-PKG-04 (Japanese spacing for Raikiri), Q-PKG-03 (print
media), P2-CF-09 (Chrome-equivalence check in CI), P2-PKG-05, E3-01c, Q-GFX-04, Q-GFX-03,
Q-CF-02, P2-CF-07, P2-CF-08, P2-CF-11, P2-RT-11, P2-DX-05, P2-ORC-03, and Q-ORC-03 at a
moment with no open PR. Checkpoint-3 evidence by Sun 15 Nov; digest Sun 15 Nov 20:00 JST.

## Earlier (Thu 12 Nov 2026, 07:40)

Session cse_011c8hfpYaknCZDyxDjnJSQG holds the fleet (Kay, 11 Nov: full authority; directive: it
works, prints on par with Chrome-based tools, faster, less memory). Every agent runs on Fable 5.2.
Nothing ran from 21:40 on 11 Nov to 07:25 on 12 Nov: the usage limit stopped the agents and the
scheduled resume task is paused (not by the fleet). Kay said "continue" at 07:25.

- 08:20: #182 (P2-CF-10) merged: the nightly ran the tool tests before it installed poppler.
  The run on this push shows whether `nightly-extra` is green again; read it before the next merge.
- (07:40) **Main was red** at d4f30d14 in one job, `linux-extra (nightly-extra)`: two tests of
  fixtures/tools/test/printers.test.mjs from #177 fail on the GitHub runner. Every other job is
  green, including arm64, ASan and soak on the merged #178. P2-CF-10 (CF agent) is on it, first.
- **Standing permissions from Kay (11 Nov 21:22 and 21:27):** merges on green CI at the exact
  head plus a fresh review; third-party printers on the fleet machine for comparison; history
  rewrite to his GitHub noreply address at a time the orchestrator picks (Q-ORC-03); merged
  branches are not deleted.
- **Open PRs:** #179 E3-00 (blocked by review; fixes pushed at 439a48d8, to be rebased on main
  and re-reviewed), #180 Q-PKG-02 (blocked by review; fixes in progress in wt/q-pkg-02).
- Running: CF agent (P2-CF-10, then Q-CF-03 Raikiri and other printers), PKG agent (#180),
  DOM agent (#179 rebase, then P2-DOM-08: branch p2-dom-08 has one local commit).
- Kay's queue: report the private findings upstream (three files in the Claude project) and
  file the docs/upstream drafts; turn the scheduled resume task back on if he wants night work.

Next session, first: `scripts/fleet/ci-status.sh $(git rev-parse origin/main)`; main must be
green before any other merge (P2-CF-10's PR is the exception: it is the fix). A new session
can't resume this session's agents: continue each task from its pushed branch or its worktree
under /home/claude/wt/. Then fresh reviews of #179, #180, P2-CF-10 and P2-DOM-08; merge as
local merge commits by the fleet identity. Then by value for the directive: E3-01a (canvas and
svg sizes), Q-PKG-03 (print media), P2-CF-09 (Chrome-equivalence check in CI), P2-PKG-05,
Q-ORC-01's decision on printer patches after Q-CF-03, Q-GFX-03, Q-CF-02, P2-CF-07, P2-CF-08,
Q-GFX-02, P2-RT-11, P2-DX-05, P2-ORC-03, and Q-ORC-03 at a moment with no open PR.
Checkpoint-3 evidence by Sun 15 Nov; digest Sun 15 Nov 20:00 JST.

## Earlier (Wed 11 Nov 2026, 21:35)

Session cse_011c8hfpYaknCZDyxDjnJSQG holds the fleet (Kay, 10:52: full authority; directive: it
works, prints on par with Chrome-based tools, faster, less memory). Every agent runs on Fable 5.2.

- **Kay's answers at 21:22:** merges allowed (green CI at the exact head plus a fresh review);
  third-party printers may be installed on the fleet machine for comparison; rewrite main's
  history to replace his work address with his gmail (Q-ORC-03, waits for his confirmation of
  the address and a moment with no open PR).
- **Merged 21:30:** #174, #175, #178 (merge commits), #176, #177 (squashes). Read full.yml on
  the pushed head before the next merge. P2-RT-10 stays on watch for a week of nightlies.
- **Open PRs, both blocked by review, authors fixing:** #179 E3-00 layout prototype (sheets must
  be screened before the style engine; one expectation from our own output; wording to make
  neutral), #180 Q-PKG-02 printer-safe output (26 of 187 pages differ in Chrome: shadow DOM,
  `-webkit-border-*`, bidi, ruby; one loop without a deadline check). P2-DOM-08 (selector cut
  in sheet.rs, a print-corrupting bug on main) follows on its own branch from the DOM agent.
- Running: DOM agent (#179 fixes, then P2-DOM-08), PKG agent (#180 fixes), CF agent (Q-CF-03).
- Kay's queue: confirm the gmail address for the rewrite; report the private findings upstream
  (5 Nov file and the two 11 Nov files in the Claude project) and file the docs/upstream drafts.

Next session, first: `scripts/fleet/ci-status.sh $(git rev-parse origin/main)`; a red main comes
before everything. Then: fresh reviews of #179 and #180 once their authors push (new sessions
can't resume this session's agents: continue from the pushed branches). Merge on green plus
approval, as local merge commits by the fleet identity (squash where a branch's history must
not reach main). Then by value for the directive: E3-01a (canvas and svg sizes), Q-PKG-03
(print media), P2-CF-09 (Chrome-equivalence check in CI), Q-ORC-01's decision on printer
patches after Q-CF-03, Q-GFX-03, Q-CF-02, P2-CF-07, P2-CF-08, Q-GFX-02, P2-RT-11, P2-DX-05,
P2-ORC-03. Push docs to main once per round. Checkpoint-3 evidence by Sun 15 Nov; digest Sun
15 Nov 20:00 JST.

## Earlier (Wed 11 Nov 2026, 20:45)

Session cse_011c8hfpYaknCZDyxDjnJSQG holds the fleet (Kay, 10:52: full authority; directive: it
works, prints on par with Chrome-based tools, faster, less memory). Every agent runs on Fable 5.2.
The usage limit has stopped the agents three times today; they are resumed in place each time.

- **Main is green** at 08944100, the second full.yml in a row with every job green.
- **Merges wait for Kay's explicit OK in this session.** Approved with green CI on the exact head:
  #174 (d627c628), #175 (0d7494ad), #176 (b1c2ed91, squash), #177 (a139b2c6, squash),
  #178 (4e3b8b3b). After merging #176: "five of the eight sizes" is seven
  (docs/upstream/fulgur-layout-gaps-0.40.md:129).
- Running: Q-PKG-02 printer-safe output (branch q-pkg-02 pushed, 3 commits, PR not open yet);
  #179 E3-00 cleanup (cost note made private, `<ol start=0>`), then a fresh review.
- **Blocked on Kay:** merges; installing and running third-party printers on the fleet machine
  (`fulgur-raikiri` for Q-CF-03, ironpress, rustyprint, forme); his work email as author of 167
  public merge commits; filing docs/upstream drafts and the private findings (5 Nov and 11 Nov
  files in the Claude project).
- Results today: fulgur 0.41.0 prints exactly as 0.40.0 (#177); every print gap root-caused
  (#176); real layout works behind an internal switch (#179: 1.5 ms per layout at 100 nodes,
  70 ms at 10,000, 3 to 6 KiB per node held); a real race in the time limit fixed (#178).
  New print gaps from #179: `@media print` never applies in fulgur, `line-height: normal`
  is 1.2, replaced elements aren't sized by attributes (Q-PKG-03, Q-GFX-03, E3-01a).

Next session, first: `scripts/fleet/ci-status.sh $(git rev-parse origin/main)`. Then: if Kay has
allowed merges, merge the five approved PRs (heads above; re-check each head and its CI first;
#176 and #177 by squash), then P2-ORC-03. Review #179 and Q-PKG-02's PR with fresh reviewers.
Then by value for the directive: E3-01a (canvas and svg sizes), Q-PKG-03 (print media),
Q-ORC-01's decision on printer patches (needs Q-CF-03 if Kay allows it), Q-GFX-03, Q-CF-02,
P2-CF-07, P2-CF-08, Q-GFX-02, P2-RT-11, P2-DX-05. A new session can't resume this session's
agents: continue each task from its pushed branch with a new agent, never from scratch. Push
docs to main once per round. Checkpoint-3 evidence by Sun 15 Nov; digest Sun 15 Nov 20:00 JST.

## Earlier (Wed 11 Nov 2026, 19:45)

Session cse_011c8hfpYaknCZDyxDjnJSQG holds the fleet (Kay, 10:52: full authority; directive: it
works, prints on par with Chrome-based tools, faster, less memory). Every agent runs on Fable 5.2.
The usage limit stopped all agents twice (11:40 to 14:10, about 15:10 to 19:10); they were resumed
in place each time.

- **Main is green** at ef2eaa30 (every full.yml job). The two failures at 3b4303ae (arm64
  `forks_never_hang`, ASan time limit) did not repeat on the same code; P2-RT-10 is still finding
  the cause (branch p2-rt-10, 2 commits: a deadline reported before its flag was up; the stress
  run now says why a child failed).
- **Merges wait for Kay's explicit OK in this session** (the safety check refuses them). Approved
  and green: #174 (d627c628), #175 (0d7494ad). Approved, CI to confirm: #176 (b1c2ed91, squash).
  Blocked by review, author fixing: #177 (q-cf-01).
- Running: Q-PKG-02 printer-safe output (wt/q-pkg-02, uncommitted work), E3-00 layout spike
  (wt/e3-00, 3 commits; private Blitz panic notes in .private-tmp/), P2-RT-10, #177 fixes.
- Findings today: fulgur 0.41.0 prints exactly as 0.40.0; every print gap is root-caused
  (docs/upstream, #176); workarounds alone pass d3-charts and noscript-message on stock fulgur;
  fulgur's maintainer is building a replacement layout engine (Raikiri).
- Kay's queue: allow merges; may the fleet install other printers (ironpress, rustyprint, forme);
  his work email is the author of 167 public merge commits (GitHub "Keep my email addresses
  private"; history needs a force-push only he can order); file docs/upstream drafts and the
  private findings.

Next session, first: `scripts/fleet/ci-status.sh $(git rev-parse origin/main)`. Then, if the
agents above are not running (a new session can't resume them): continue each task from its pushed
branch or worktree under /home/claude/wt/ with a new agent, never from scratch. After their
reports: fresh reviews, Q-ORC-01's decision (which printer fixes we carry), then Q-CF-03, Q-CF-02,
P2-CF-07, P2-CF-08, Q-GFX-02, P2-DX-05. Push docs to main once per round: every push restarts
full.yml. Checkpoint-3 evidence by Sun 15 Nov; digest Sun 15 Nov 20:00 JST.

## Earlier (Wed 11 Nov 2026, 11:10)

**Kay handed the fleet to session cse_011c8hfpYaknCZDyxDjnJSQG at 10:52 with full authority and one
directive: make it work, print on par with Chrome-based tools in quality, be faster and use less
memory** (DECISIONS 2026-11-11). Hard stops are unchanged.

- Done since 7 Nov (all merged on 11 Nov, 07:40 to 10:49): #166 P2-DOM-07, #167 P2-CF-06, #168 P2-GFX-01, #169 P2-PKG-04,
  #170 E2-11 musl gems, #171 P2-RT-07 (engine patch level 10), #172 P2-RT-09, #173 P2-DX-03.
  No commits between 7 Nov 01:59 and 11 Nov 04:32.
- Phase 2 targets, all met at 22153052 (COMPAT.md): timers 91.2%, dom/nodes 92.8%, dom/events and
  domparsing above target. Real-world pages: DOM equal to Chrome's; print 2 of 6, the other 4 fail
  in fulgur 0.40.0 alone (fixtures/checkpoint-3/NOTES.md).
- In progress: #174 P2-DX-04 in review. Quality track started: Q-CF-01 printer matrix (CF),
  Q-GFX-01 print-gap root causes (GFX), E3-00 layout design spike (DOM).
- Main 5a3aa25f: full.yml and ci.yml were running at 11:00; read them before the next merge.
- 11:35: Q-GFX-01 back (#176, needs review): every print gap root-caused; fulgur 0.41.0 fixes none;
  workarounds on our side alone make 4 of 6 real-world pages pass. Q-PKG-02 (printer-safe output)
  and Q-ORC-02 (fulgur's second layout backend, raikiri) started.
- 11:50: the Fable 5.2 usage limit stopped the agents (resets 14:10 JST). Their work is in their
  worktrees under /home/claude/wt/ and their branches: q-cf-01 (fulgur 0.40 vs newest; was at its
  tests), q-pkg-02 (printer-safe output), e3-00 (layout spike), and the review of #176. This session
  resumes them at 14:13; a new session that finds them stopped starts the same task ids again from
  the pushed branches (list remote branches first).
- 14:20: limit reset; the four agents were resumed in place (same worktrees). **Main is red** in
  full.yml at 3b4303ae (docs-only commit): `test (ubuntu-24.04-arm)` on core's `forks_never_hang`
  (1 child of 3000 failed, none hung; the job passed on 90427b9e) and `linux-extra (asan)` on
  `limits_are_typed_and_the_renderer_carries_on` (time limit). P2-RT-10 (RT) is on both.
  Every push to main cancels the running full.yml: batch docs pushes, one per merge round.
- Open PRs: #174 (approved, green), #175 P2-ORC-02 root pins (from the previous session; needs a
  fresh review), #176 (in review).
- **Blocked on Kay: merges.** The session's safety check refuses fleet merges until he allows them
  in this session. Waiting: #174 (approved, green), #176 (in review). Kay's queue (no deadline): file the docs/upstream drafts and the private
  findings; confirm the employer open-source check.

Next session, first: `scripts/fleet/ci-status.sh $(git rev-parse origin/main)`; fix a red main before
anything else. Then merge #174 if approved; collect Q-CF-01, Q-GFX-01 and E3-00 reports and write
Q-ORC-01's decision; start Q-CF-02 (page-path benchmark) and Q-PKG-01's design. Leftovers after that:
P2-ORC-02 root pins, P2-PKG-01, E2-14, P2-DOM-06, E2-10. Checkpoint-3 evidence ready by Sun 15 Nov;
digest Sun 15 Nov 20:00 JST.

## Earlier (Sat 7 Nov 2026, 01:30)

Session cse_011c8hfpRCVhwbjSCxu8U5UF holds the fleet. Merged since 23:00: #159 #162 #163 #164 #165. No PR is open. Phase 2's SPEC targets are met on our baseline (COMPAT.md at e879ec95): timers 91.2%, dom/nodes 92.8%, dom/events 75.7% (66.7% on wpt.fyi's count of 848: state both at checkpoint 3), domparsing 61.6%. Real-world pages: DOM 5 of 5 equal Chrome; print 2 of 6 pass, the other 4 equal fulgur printing Chrome's own DOM.

Next session, first: read full.yml on main (ruby and native-gems were red on about.toml until #159; ASan on time scales). Then in order: P2-CF-06 (rerun traversal and css/cssom, README WPT numbers via Docs), P2-DOM-07, P2-ORC-02 root pins, P2-RT-07 libm, E2-11 musl gems (third to cut), the follow-up ASan test that forks with the parent idle. Prepare the checkpoint-3 message (Sun 22 Nov 11:00) from docs/guide/checkpoint-3.md, COMPAT, fixtures/checkpoint-3, soak and determinism results. Digest due Sun 8 Nov 20:00 JST.

## Earlier (Thu 5 Nov 2026, 11:45)

Session cse_011c8hfoqJ45246YxjoyFzH7 holds the fleet. Merged today: #97 #125 #126 #127 #128 #129 (engine patch level 9). Open: #104 (needs limit errors from promises, via #127), #105 (delta approved; contains #104), #123 (WPT, approved; base e2-04b).

## Earlier (Wed 4 Nov 2026, 14:50)

**Main is green**: full.yml passed all 14 jobs at 0e67e98c (ASan leak fixed in #111, macOS flakes in #112,
symbolized ASan in #107). Merge freeze lifted. Also merged: v0.1 README/CHANGELOG (#110, #113) and the
Ruby 3.2 signal fix (#114). **v0.1.0 release PR #38 merged** (915d5cbf; pre-release review approved, 15/15 checks). Kay tags
v0.1.0 and pushes the five gems following docs/RELEASE.md. After #38: phase 2 starts
with MSRV 1.89, then the open phase-2 PRs (#88 #90 #92–#105) and #106.

## Earlier (Wed 4 Nov 2026, 13:05)

**Checkpoint 2 signed off by Kay at 12:57** (all questions approved; see DECISIONS). The live session
(cse_011c8hfp9aeYhQp5RpWyywaN) took the fleet back at 12:58. **Merge freeze until main's full.yml is
green**: linux-extra-asan reports a 1.1 MB leak at exit of the core lib tests (P1-RT-42) and test-macos-14
has timing flakes (P1-RT-43); P1-PKG-17 symbolizes ASan traces in CI. Then: v0.1 release (#38: main merged
in, dry-run, fresh review, merge; Kay tags/publishes), then phase 2 starting with MSRV 1.89 (MSRV 1.89 waits until #38 has merged, so the release docs and dry-run keep saying 1.87).

## Earlier (Wed 4 Nov 2026, 04:40)

Session cse_011c8hfnWG3Q5HtHFaa21vge resumed the fleet after 55 h with no heartbeat. No reply from Kay
to checkpoint 2 visible here (no comments or reviews from him on #38). #82 (P1-DOM-04) and #83 (P1-RT-29)
were merged on Sun evening. main adfe71a7: 13 of 14 full.yml jobs green; asan red (P1-DOM-05, a test's
2 s limit); reference red (P1-CF-30, no browser). Phase-2 internal scaffolding starts now under the
DECISIONS 2026-11-04 rule; nothing public until sign-off. macos-14 still ran on 2 Nov.

## Earlier (Sun 1 Nov 2026, 16:00)

**Phase 1: islands. Checkpoint 2 message sent to Kay at 15:58 JST** (SendUserMessage + push), in session
cse_011c8hfoBxyki5Uo3FJx7LZe. Waiting for his 5 answers: (1) engine distribution (recommend gem + CLI
v0.1, Rust crates held), (2) memory target: a) ≥5× PSS or b) keep 10× + COMPAT gap, (3) MSRV 1.89 at
phase 2, (4) outline labels default OK, (5) publish v0.1? Also asked him to copy docs/ci/*.yml into
.github/workflows before macos-14 retires on 2 Nov. The message covers today's 20:00 digest.

This session merged (each on green ci/linux + a fresh reviewer's APPROVE): #72 (RT-35 macOS preload
flake), #73 + #77 (RT-27 ASan time scale), #74 (DOM-03), #75 + #79 (BUILTIN_RENDERERS), #76 (PKG-01d),
#78 (GFX-06 stylo thread-local leak in a test), #81 (RT-23 regexp per-match polls, engine patch level 4),
#80 (DOM-02 id-handle spike, internal; first review blocked, fixed). main abd31b70: full.yml all 14 jobs
green incl. asan and macos-14. #38 head f0d7091f has main up to abd31b70 (not #80/#81).

### Checkpoint 2 evidence (all on main)
- Fixtures: fixtures/checkpoint-2/index.md (+ NOTES.md). Gate 12/12 islands pass; 34/35 comparisons
  pass; the one fail is vegalite-line-ja printed by fulgur, shape 10.2% vs 10% at the 10 px "6月"
  label (Chrome-printed 4.8%, page 7.3%). Determinism 11/11 byte-identical across processes, TZ and
  locale. Mention: local Chromium differs from CI by 65 px / 38 px on two ECharts fixtures; refs and
  fulgur floor are CI's (NOTES.md). Results predate #63 (no render-path change).
- Hostile: 59 cases, 60 results, all as the manifest expects; no crash, no hang (COMPAT.md).
- Bench (ci-results runs/15b530e0…/bench/grover/bench/summary.md; not committed to bench/):
  Grover ÷ surimono+fulgur Ruby: 2.4× cold / 2.4× warm (2× met); PSS 5.8× / 5.6× (10× not met;
  fulgur alone with Chrome's SVGs is only 8.7×). Rust: 3.0×/2.4×, PSS 8.0×/8.2×. Against
  chrome-headless-shell: 1.8–2.3× time, 3.8–5.2× PSS.
- Release PR #38 (release/v0.1.0, head 715a08f4): dry-run passes except the engine distribution
  (crates.io users get unpatched rquickjs-sys; link fails). Licence files fixed. Source gem and the
  x86_64-linux native gem build and pass. Needs main merged in again before the message.
- Kay decisions (docs/fleet/PENDING_DECISIONS.md "Open for checkpoint 2"): engine distribution
  (recommend v0.1 = gem + CLI from source, Rust crates held; publish the fork under our own names
  later), the 10× memory target (PSS vs summed RSS; fulgur's own floor), MSRV 1.89 at phase 2,
  outline text (labels not selectable), macos-14 retirement → YAML change (docs/ci/), reference.yml
  paths should include crates/gfx/**.

### Open PRs
| PR | Task | State |
|---|---|---|
| #38 | v0.1 release PR | waits for Kay; merge main in again (gets #81) before any merge |
| #84 | P1-RT-18 | ci/linux green, in review |

### Blocked / waiting on Kay
- Checkpoint 2 answers (above). Don't merge #38 or any phase-2 public API without them.

### Next (in order; a resumed session starts here)
1. #104 fix, delta review, merge; #105 merge main, CI, merge.
2. Retarget #123 to main, regenerate COMPAT (command in #123), merge. Then P2-CF-02, E2-13, E2-06/07/08, E2-10 surimono side, E2-12. Checkpoint 3 is Sun 22 Nov 11:00.
3. Sun 8 Nov 20:00 JST: five-line digest to Kay.
4. Each push to main cancels the running full.yml; batch docs commits. Read full.yml on main under Kay's adopted workflows (macos-15).

### Disk
When free space drops under ~5 GB, run `scripts/fleet/prune-target.sh`.

### Branch rule
Never rebase or force-push a pushed branch; fix with new commits; downstream branches merge their parent in. PRs merge with merge commits. The orchestrator may merge main into an approved branch when the result is exactly the mechanical merge.
