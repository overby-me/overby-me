# Where a PDF puts its spaces, and how we decide

A PDF does not have to write spaces. It positions text, and a word break may be
nothing more than the pen starting a little further right than it stopped. So
the reader has to decide, for every gap between two draw calls, whether it is a
space or the ordinary jitter of a producer emitting a line in pieces.

`WORD_GAP` in `src/pdf_text.rs` is that threshold, as a fraction of the em. It
is compared against the pen's true end, computed from the advance widths, not
against a guess from the character count: guessing produced "G eneralsekretæren".

## Why it was wrong at 0.20

The samværspolitik is justified, and a justified line compresses its word
spaces to make the margins meet. Two of them came out at **0.187** and **0.198**
of the em, just under the threshold, and the reader ran the words together:

    Der eraltid nogen at gå til, hvis duhar brug for at tale

Nudging the constant until that one document reads correctly is how a threshold
becomes folklore. It was measured instead.

## The measurement

`scripts/../scratchpad/gapsweep.nu` (kept out of the tree; the method matters
more than the script) runs the extractor over a corpus at a range of thresholds
and diffs the words against **poppler's** `pdftotext`. Poppler is the oracle for
words only, not for layout: it has decades of handling for exactly this
question, and no stake in how this renderer arranges blocks. Tokens are compared
as multisets, so a difference means one reader split or joined a word the other
did not.

Seventeen documents from the wiki, 57 264 words:

| gap  | differing words |
|------|-----------------|
| 0.20 | 1975            |
| 0.18 | 1821            |
| 0.16 | 1809            |
| 0.14 | 1767            |
| 0.12 | 1746            |
| 0.11 | 1691            |
| 0.10 | 1688            |
| 0.09 | **1667**        |
| 0.08 | **1667**        |
| 0.06 | 1719            |
| 0.03 | 1724            |

A U with a flat bottom at 0.08–0.09. Above it, real spaces are missed and words
run together. Below it, the gaps inside words start counting as spaces and words
come apart, which the rise at 0.06 is.

**0.09** is the value, at the minimum and one step further from the cliff below
than 0.08 is. The absolute count is not a score to chase: much of it is the
folios this reader removes and poppler keeps, and the leader dots a contents row
drops. Only the differences between rows mean anything.

## Re-running it

Set `PDF_WORD_GAP` when running the harness and the constant is overridden, so
a sweep needs no rebuild:

    PDF_UNDER_TEST=/path/to.pdf PDF_DUMP=1 PDF_WORD_GAP=0.09 \
      cargo test pdf_text::harness::dump_the_words -- --nocapture

Worth redoing if the corpus changes character: these documents are Word exports
and Google Docs exports of Danish prose, and a typesetter's PDF spaces its words
differently.
