## What this changes, and why

<!-- Link the issue it came from, like "Closes #12". Anything big should have one: see CONTRIBUTING.md. -->

## How you tested it

<!-- The commands you ran. For anything that touches a radio: the device, its firmware, the host, the sample rate and what you did. Not tested on hardware? Say so. That is allowed; pretending is not. -->

## Checklist

The rule behind each line is in [CONTRIBUTING.md](https://github.com/musithang/sdrtop/blob/main/CONTRIBUTING.md).

- [ ] `fmt`, `clippy`, the tests and the 1.88 check all pass
- [ ] The signal processing and decoding are ours, with no external crate (rule 1)
- [ ] No new dependency, or the issue where we agreed on it is linked (rule 2)
- [ ] Nothing unknown is shown as a number (rule 3)
- [ ] The hardware is named above, or the change says it is unverified (rule 4)
- [ ] `user_docs/` and `CHANGELOG.md` are updated, if a user can see this (rule 5)
- [ ] New `.rs` files start with the SPDX line
- [ ] AI assistance: yes / no (rule 7)
