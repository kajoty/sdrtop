## What this changes, and why

<!-- Link the issue it came from, like "Closes #12". Anything big should have one: see CONTRIBUTING.md. -->

## How you tested it

<!-- The commands you ran. For anything that touches a radio: the device, its firmware, the host, the sample rate and what you did. Not tested on hardware? Say so. That is allowed; pretending is not. -->

## AI assistance

<!-- "None", or which tool and what for. Either answer is fine; not saying is not. -->

## Checklist

Tick what applies; a line that doesn't apply to your change can just be deleted. Each links to the rule behind it in CONTRIBUTING.md.

- [ ] [The four checks CI runs](https://github.com/musithang/sdrtop/blob/main/CONTRIBUTING.md#before-you-open-the-pull-request) pass: `fmt`, `clippy`, the tests, and the Rust 1.88 check
- [ ] [Signal processing and decoding are our own code](https://github.com/musithang/sdrtop/blob/main/CONTRIBUTING.md#1-sdrtop-decodes-signals-itself), no new crate doing it
- [ ] [No new dependency](https://github.com/musithang/sdrtop/blob/main/CONTRIBUTING.md#2-a-new-dependency-needs-an-issue-first), or the issue where we agreed on it is linked
- [ ] [Nothing unknown is shown as a number](https://github.com/musithang/sdrtop/blob/main/CONTRIBUTING.md#3-never-invent-a-number)
- [ ] [The hardware is named above](https://github.com/musithang/sdrtop/blob/main/CONTRIBUTING.md#4-say-what-you-tested-and-on-what), or the change says on screen and in the docs that it is unverified
- [ ] [`user_docs/` and `CHANGELOG.md` are updated](https://github.com/musithang/sdrtop/blob/main/CONTRIBUTING.md#5-the-docs-change-with-the-code), if a user can see the change
- [ ] New `.rs` files start with `// SPDX-License-Identifier: GPL-3.0-or-later`
