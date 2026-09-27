# The `sdrtop` SigMF Extension Namespace

← [Back](README.md)

This file defines the `sdrtop` extension namespace, version 1.0.0, as SigMF
1.2.6 section 1.9.2 asks: every field sdrtop writes outside `core` is described
here. The namespace is listed in `core:extensions` with `optional: true`, so a
tool that has never heard of sdrtop can ignore all of it and still read the
recording correctly.

The recordings themselves are ordinary SigMF: a `.sigmf-data` file holding the
samples exactly as the radio delivered them (`ci8` from a HackRF, `cu8` from an
RTL-SDR, `ci16_le` through SoapySDR) and a `.sigmf-meta` file beside it.

## 1 Global

| name | required | type | description |
|---|---|---|---|
| `sdrtop:full_scale` | true | double | The sample value that means full scale. Divide by it to get samples in the range -1 to 1, the way sdrtop reads them. For `cu8` the zero is 127.5 and the value is measured from there. |
| `sdrtop:bits` | true | uint | How many bits of the container the converter fills. A `ci16_le` recording from SoapySDR is often a 12-bit converter in a 16-bit container. |
| `sdrtop:stack` | false | string | The software between sdrtop and a radio with no firmware of its own, with its version, for example the SoapySDR runtime. Absent on a radio that has firmware. |
| `sdrtop:gain` | true | string | The gain chain when the recording started, by stage name as the radio's driver names them (`LNA=16,VGA=20`), and the front end boost when the radio has one. Changes during the recording are annotations labelled `gain`. |
| `sdrtop:reference` | true | string | The frequency reference sdrtop held when the recording started, or that it held none. **The samples are never corrected by it**: it is here so a reader who wants absolute frequencies has what they need. |
| `sdrtop:iq_correction` | true | string | The DC block and I/Q correction sdrtop was showing on screen. **The file never has them applied**; this says what the panels saw, so both stories can be told from one recording. `none` when neither was on. |
| `sdrtop:state` | true | string | `recording` while the file is still growing, `finished` once it has stopped. The metadata is rewritten once a second, so a file left behind by a crash says `recording` and is complete up to its last second. |
| `sdrtop:stopped` | false | string | Why the recording ended, as a sentence. Present once `sdrtop:state` is `finished`. |
| `sdrtop:pairs_written` | true | uint | Samples in the `.sigmf-data` file. |
| `sdrtop:lost_driver` | true | uint | Samples the radio's driver reported it dropped. |
| `sdrtop:lost_queue` | true | uint | Samples sdrtop could not hand to the disk in time, because the queue in front of it was full. |
| `sdrtop:lost_unexplained` | true | uint | Samples missing from the stream that nobody reported. Normally zero; counted, not assigned to a cause. |

## 2 Captures

No `sdrtop` fields. How sdrtop uses the `core` ones:

- A new capture segment starts at the beginning, after every stretch of
  missing samples, and at every retune. `core:global_index` is the sample's
  position in the stream the radio delivered, counted from the first sample
  offered to the recording, so it runs ahead of `core:sample_start` by
  exactly what is missing so far.
- `core:frequency` is the frequency sdrtop had set when the block arrived.
  The radio's own settling after a retune is not measured and not in the
  file.
- `core:datetime` of the first segment is the system clock when the first
  block reached sdrtop, which is after the driver's own buffering: late by up
  to one block. Every later segment's time is the first one's plus its
  `core:global_index` divided by the sample rate, so the segments agree with
  each other to the sample, and with the wall clock only as well as the
  first one does.

## 3 Annotations

No `sdrtop` fields. Every annotation sdrtop writes marks a moment, so each
has `core:sample_count` 0, and `core:generator` is `sdrtop`. The labels:

| `core:label` | at | `core:comment` says |
|---|---|---|
| `lost N` | the first sample after a hole | how many samples are missing, how long that is, and how many of them the driver dropped, the queue refused, or nobody explained |
| `tuned F MHz` | the first sample at a new frequency | that the radio's settling is not in the file |
| `gain` | the first block after a gain change | the new chain, set by the time the block arrived, not necessarily applied by the radio at that sample |
| `stopped` | the end of the file | why the recording ended |
