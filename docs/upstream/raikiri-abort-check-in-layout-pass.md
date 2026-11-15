# Raikiri: an abort check inside the layout pass

Draft for Kay to file with Raikiri's maintainers (the fulgur project), as a
public feature request; it describes no defect. Against raikiri-dom 0.0.6 and
raikiri-html 0.0.6, the versions surimono pins behind its hidden `raikiri`
feature. No local `[patch]`: surimono works around it by running the layout on
a thread of its own that the caller stops waiting for (`crates/dom/RAIKIRI.md`,
section 10), which bounds the call but not the layout.

## Problem

`PageLayoutControl::with_abort_check` stops a layout only between stages. A
caller that lays out untrusted HTML under a time limit can't stop the source
layout pass, which for a one-page document is all of the layout. The check is
looked at in pagination's loops and in the projection, never inside the pass,
so for a caller that lays a document out as one long page its granularity is
the whole layout.

## Proposal

The `LayoutSession` keeps the control's abort check and looks at it as it lays
out boxes, in the taffy adapter's child layout and in the paragraph builder,
every few hundred boxes or lines (a check is the caller's closure: an atomic
load and a clock read). Once it returns true the session is aborted: every
later child layout returns an empty layout at once without measuring, so the
pass unwinds in microseconds without a `Result` through taffy's API, and the
existing check after the pass (`check_page_index`) turns it into
`LayoutError::Aborted`.

A layout that isn't aborted is unchanged.

## Test

Lay out a large document with a check that turns true after N calls and expect
`LayoutError::Aborted` within a bounded number of further box layouts.

## What it would let surimono drop

The helper thread per layout and its process-wide cap of four live layout
threads (`MAX_LAYOUT_THREADS` in `crates/dom/src/layout_raikiri.rs`), and with
them the case where a caller has stopped waiting for a layout that still runs.
