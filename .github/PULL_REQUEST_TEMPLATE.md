## What this changes

<!-- The behavior a person using BoBe will notice, or "no user-visible change". -->

## Why

## How it was checked

<!--
Checks that follow from the change. Unit tests alone are not evidence that
providers, streaming, tools, persistence or recovery work in the running
product: say what you ran through the real `bobe` daemon or the app.
-->

- [ ] `scripts/tasks/check.sh`, or the parts of it this change touches
- [ ] Operated through the real daemon, app or body where behavior changed

## Notes for the reviewer

<!-- Database schema, wire or contract changes, new dependencies, and anything you are unsure about. -->

- [ ] Rust and Swift wire constants changed together, if any changed
- [ ] No prose comments added to source; constraints are in the docs
- [ ] No secrets, credentials or personal data in the diff
