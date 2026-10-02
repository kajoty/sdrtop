# Contributing to sdrtop

Thank you for your interest in contributing. Bug fixes, hardware reports,
documentation improvements and new features are all welcome.

This document describes the rules every contribution must follow and the steps
for getting a pull request merged. The principles behind these rules are
described in [POLICY.md](POLICY.md).

---

## Before you start

- **Small changes** (bug fixes, typos, documentation corrections, test results
  from your hardware) can be submitted directly as a pull request.
- **Larger changes** require an issue first. This includes new panels, new
  decoders or demodulators, new device backends, new dependencies, and any change
  to how a measurement is calculated or presented. Describe what you want to
  build and how you plan to approach it, and wait for agreement before starting
  the work.

Pull requests for larger changes that were not discussed first may be declined,
even if the work itself is good.

---

## Project rules

### 1. Signal processing and decoding are implemented in sdrtop itself

sdrtop decodes signals with its own code. Every part of the signal chain
(filters, oscillators, demodulators, symbol timing, error detection, protocol
decoding) must be implemented in this repository.

The only exceptions are:

- **`rustfft`**, for the forward FFT
- **`num-complex`**, for the complex number type

External crates must not be used for signal processing or decoding. This applies
to `[dev-dependencies]` as well: test references must be written from the
relevant specification, not taken from another implementation. The Bluetooth
receiver, for example, is tested against a reference transmitter and tester
written from the Bluetooth SIG's own test specifications.

**Why:** sdrtop is a measurement instrument. A result can only be verified and
explained if the code that produced it is part of the project. See
[Why write it out](user_docs/demodulator.md#why-write-it-out).

### 2. New dependencies require approval

Adding any crate to `[dependencies]`, `[dev-dependencies]` or
`[build-dependencies]` requires an issue and maintainer approval before the pull
request. A new dependency is accepted only when there is no reasonable way to do
the job without it. Dependencies that perform signal processing or decoding are
not accepted (see rule 1).

### 3. Never display an invented value

If a value cannot be measured or obtained from the device, it must be shown as
unavailable. Do not substitute a default, an estimate or a zero for an unknown
value. Every label must describe what was actually measured. See
[Rule Two](POLICY.md#two).

### 4. State what was tested, and on what hardware

Pull requests that affect hardware behaviour must state the device, its
firmware, the host system, the sample rate and the steps taken. Functionality
written from a datasheet or specification but not tested on real hardware is
acceptable only if it is marked as unverified in the interface and in the
documentation.

### 5. Update the documentation in the same pull request

Any user-visible change must:

- update the relevant page in [`user_docs/`](user_docs/README.md), and
- add an entry under `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md).

Each fact is documented on one page only; other pages link to it. Edit the page
that owns the topic instead of describing it again elsewhere.

You don't need to match the writing style of the existing documentation. Clear
and correct is enough; the maintainer may edit the wording before release.

Do not change `user_docs/whats-new.md`, the version number or the release
files. These are updated by the maintainer as part of the
[release process](RELEASING.md).

### 6. Out of scope

sdrtop does not and will not support audio output, transmitting, or mouse
input. Pull requests adding any of these will be declined. See
[Rule Eight](POLICY.md#eight).

### 7. Disclose AI assistance

AI tools may be used. If you used them, say so in the pull request description.
You must understand every line you submit and be able to answer questions about
it during review.

---

## Pull request checklist

All of the following must pass before a pull request is reviewed. These are the
same checks CI runs:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all --locked
cargo +1.88 check --locked --all-targets   # minimum supported Rust version
```

In addition:

- [ ] New `.rs` files start with `// SPDX-License-Identifier: GPL-3.0-or-later`.
- [ ] Commit messages follow the format `type(scope): summary`, in lowercase,
      for example `fix(soapy): keep the remote address in the open markup`. The
      commit body explains why the change was made.
- [ ] The pull request covers one topic. Large changes are split into smaller,
      independently working pull requests.
- [ ] The description explains what changed, why, and how it was tested.
- [ ] No new dependency, or the issue where it was approved is linked.
- [ ] Documentation and changelog are updated (rule 5).
- [ ] AI assistance is disclosed, if used (rule 7).

---

## Reporting bugs and hardware results

Open an issue. What to include is listed under
[Getting help](user_docs/troubleshooting.md#getting-help); the log file is the
most important part.

Reports from radios the project has not been tested on are especially valuable,
whether the device works or not.

---

## Review process

All pull requests are reviewed by the maintainer. sdrtop is maintained in spare
time, so a review may take several days. If a pull request cannot be merged as
submitted, the review will explain why and which parts can be kept.

Contributors are credited in [CREDITS.md](CREDITS.md).

---

## License

sdrtop is licensed under [GPL-3.0-or-later](LICENSE). By submitting a
contribution, you agree that it is licensed under the same terms.
