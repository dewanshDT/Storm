Implement the Storm redesign round 2 in the Flutter client.

Read, in order:
1. `design_handoff_storm_v2/README.md` (the baseline)
2. `design_handoff_storm_crews/README.md` (crews)
3. `design_handoff_storm_redesign_r2/README.md` (this round; it takes precedence where they differ)

Open `design_handoff_storm_redesign_r2/Storm Redesign R2.dc.html` in a browser to see the approved frames. The `.dc.html` files are design references, not code to port. Build with the existing widgets and `lib/ui/tokens.dart`; literal colours or sizes in `lib/ui/` fail `token_conformance_test`.

Work in separate PRs, in this order. Each PR must pass `flutter test` and the README's acceptance items for its area:
1. App bar and the top alignment (§1)
2. Agents sidebar rules and its header row (§2), client side only, using the attention data that already exists
3. The decision strip, replacing the NEEDS YOU card, the delete modal, the banners and the Trash dialogs (§3)
4. The inspector icon rail, the Agents tab and the slimmed About (§3)
5. The launcher (§4)
6. Trash and retention in Vaults (§5)
7. Settings › Agents & hosts grid, Integrations, and the AI access copy (§6)
8. Phone (§7)

Server work (README "Server/API work") goes in its own PRs. Where the client needs data the server doesn't send yet, ship the UI with a clear fallback and note it in the PR.

Don't decide the README's open questions. Ask me. Don't change the constraints section.
