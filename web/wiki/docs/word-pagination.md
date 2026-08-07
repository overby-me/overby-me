# Where a Word document's pages end

A `.docx` file does not say where its pages break. Word works that out when it
draws the document, from the page size, the margins, the faces, and how the text
wraps in them. The app has to work out the same thing to offer a page control
that means anything, and it does it the only way that can be right: it lays the
document out a second time, off-screen, at the size the document says its pages
are, and reads where the content crosses each page boundary.

The reading copy cannot be measured. It is deliberately reader-relative: text at
the reader's size, lists at half Word's indent, tables sized to their contents so
they stay readable on a phone. Those are the right decisions for reading and the
wrong ones for pagination, so the measuring copy overrides them
(`.docx-measure` in `assets/style.css`) from custom properties each paragraph
carries (`measured_style` in `src/components/docx.rs`).

## What has to be true for the answer to be right

Each of these was wrong at some point, and each was worth a page or more:

- **The faces.** Word's widths, not the reader's. The app ships three
  metric-compatible substitutes and measures in whichever the document uses:
  Carlito for Calibri, Caladea for **Cambria**, Liberation Serif for Times New
  Roman. Cambria is not Times: measuring a Cambria document in Liberation Serif
  fitted five list items too many onto every page.
- **The face per PARAGRAPH.** A document whose body is Calibri and whose list
  style is Cambria is ordinary. One face for the whole document gets the other
  one wrong.
- **What single spacing means.** Word states line spacing as a multiple of the
  face's own line box, which CSS does not: Calibri's is about 1.22 times the
  size, Cambria's 1.17, Times' 1.15. Measured at runtime from the face itself
  (`set_single_spacing`), not assumed.
- **The indents.** Word's, in Word's points, not the reading copy's.
- **Spacing does not collapse.** Word ADDS the space under one paragraph to the
  space over the next; CSS keeps the larger. The measuring copy spaces with
  padding for that reason.
- **Except where the space is AUTOMATIC**, which is a margin and does collapse.
  A paragraph can decline to state its spacing and leave it to whoever draws the
  document (`beforeAutospacing`/`afterAutospacing`, ECMA-376 §17.3.1.33). Word
  writes that whenever text arrives from a browser or a Google Docs export, and
  a fifth of this wiki's documents carry it. The figure is **14pt**, it
  overrides whatever the paragraph states, and it collapses against its
  neighbour rather than adding to it — these attributes exist to reproduce a
  browser's paragraph margins, collapsing included. Both halves are measured
  (`scripts/word-pages.nu`, one rendering with the attributes and one with them
  stripped): every boundary moved by exactly 8.0px, which is 14pt standing where
  the document default's 8pt stood. Reading past the attributes left every such
  paragraph 6pt tight; adding 14pt to the 8pt already there would have been 6pt
  too loose.
- **Table columns.** The widths the document states, and a table wider than the
  text column overflows the margin as it does in Word rather than being squeezed
  into it. Left to the browser, one table was laid out 134/60/398 where Word has
  290/75/305, and every row came out short.
- **A page ends between two LINES of a paragraph**, not above it. Word leaves as
  many lines as fit and carries the rest over, keeping two on each side of the
  break — widow and orphan control, which every document here asks for. This
  file used to claim the opposite, and measuring it that way left up to a
  paragraph of empty page at every break.
- **But an element that cannot be sliced moves whole**, and then the page ends
  early with the space below it unused: a table row (unless it splits, below), a
  heading kept with what follows it, a paragraph of three lines or fewer.
  Measuring the ribbon continuously and dividing by the page height gets this
  wrong in both directions.
- **A table row is the exception**: Word carries the rest of a row onto the next
  page unless `cantSplit` forbids it, so a page that ends inside a row is full.
- **An empty paragraph is a line.** Word gives it the line of its paragraph
  mark, and half these documents space their sections with runs of them; an
  empty block in CSS is half a line or none. Four blanks before a heading came
  to a paragraph less than the page holds.
- **The space under a page's last paragraph falls off the page.** Word puts a
  paragraph on the page when its LINES fit and cuts the space after it at the
  page edge, so the fit test asks where the text ends, not where the box does.
- **The layout has to have settled.** A font's `load` resolving is not the same
  as the document having been laid out again in it. The measurement waits for
  `fonts.ready` and then measures twice a frame apart, accepting the answer only
  when the two agree — without that, the same file came out eight pages on one
  visit and nine on the next.

## Where the truth comes from

Two places, and the second one took a while to find.

**The files themselves.** Word leaves a `lastRenderedPageBreak` where it last
drew a break, and eighty-four of the wiki's documents carry them. Read with
care — see the two traps below.

**A converter, run here.** `scripts/word-pages.nu` renders a document to PDF
with LibreOffice and reports how many pages came out and what each begins with.
That needs no export from anyone and no Word licence, so a change to the model
can be checked against a real document in about a minute.

LibreOffice was written off as a second opinion once, on the grounds that it
made a document ten pages where Word makes eight. That was its **font
substitution**, not its layout: pointed at the same metric-compatible faces this
app measures in (Carlito, Caladea, Liberation Sans/Serif), the document that
read ten came out nine — which is what the reader's own export of it says — and
the break this app was getting wrong landed exactly where the reader reported
Word puts it. The substitutions are the whole trick, and the script sets them up
in a fontconfig of its own so nothing installed on the machine gets a vote.

It is a second renderer, not Word, so it is worth what it agrees with: it
matches Word on both documents here that can be checked against Word directly.
Where it and the file's own record disagree, say which one is being quoted.

## What is checked

`scripts/check-word-pagination.nu` opens five real documents in the deployed
build and compares both halves of the answer — how many pages, and what each
page begins with — against what Word recorded in the files themselves.

Four of the five match Word exactly, including both of the assembly's own. So
does `indstillinger` under Ekstraordinært landsmøde, which is checked by hand
(the test account is not a member of that context).

## What Word's own record is worth, and how to read it

Eighty-four of the wiki's Word files carry `lastRenderedPageBreak` hints, which
makes them a truth set worth measuring against — but the hints have a trap in
them. **Word writes the hint into every CELL a page boundary crosses**, so one
break across a four-column row appears four times. Counting hints as pages made
one document look like 22 pages when Word makes 11 — and made this renderer look
half as long as Word when it was exactly right. Group the hints by their
enclosing `w:tr` and count each row once.

The hints have a second trap: **they are where the document broke the last time
WORD drew it**, not where the text breaks now. A file edited in Word Online, or
saved by another tool, keeps its old hints. Two of the corpus documents — both
of them named "(opdateret)" — imply pages holding a single small table row,
which the text they now contain cannot fill. Their hints are stale and they are
useless as truth. The test that catches it: measure each of Word's pages in this
renderer's own layout, and if two or more of them hold less than half what the
median page holds, the record no longer matches the text.

Measured against the seven whose hints survive that test, placing Word's breaks
by POSITION in the flow rather than by their text (a document of twenty
identical tables cannot be placed by its text), this renderer lands within **±5%
of Word on every one**: medians of 97, 98, 98, 100, 101, 105 and 105 per cent of
a Word page. The prose-only documents come out at 100.0%.

That is the accuracy the model has, and it is worth saying plainly what it
means: five per cent of a page is about a paragraph. A document can be measured
correctly by this standard and still break one paragraph away from where Word
breaks it, which is what `posk_arbejdsprogram_21-22` did — its first page ended
about 20px past Word's, and it broke its second page a heading late.

That one turned out to be a rule the renderer did not know rather than an
accumulation of small errors: the file's paragraphs ask for **automatic**
spacing, and the renderer was giving them the document default instead. See the
entry above. It is worth recording how it was found, because the same method
finds the next one: render the file twice with the converter, once as it is and
once with one attribute stripped, and diff the two line by line. The gap between
the two renderings is what that attribute is worth, with every other variable
held still — no arithmetic on inferred quantities, and no correction fitted to a
single file.

### The one real gap: Aptos

Word 2024 changed its default font from Calibri to **Aptos**, and this app has no
metric-compatible substitute for it — Carlito stands in, and Carlito is
Calibri's metrics, not Aptos's. Every document the organisation writes from now
on is Aptos by default. The two Aptos documents in the corpus measure 20-40%
shorter here than Word and LibreOffice make them, and no correction to the line
box or the letter spacing closes it: swept to a twelve per cent wider tracking
and a twenty-two per cent taller line box, they still come up short. There is no
open font cut to Aptos's widths to ship, so this stays a known gap until there
is one.

## The hardest one, and what remains of it

`evaluering_af_fu_og_posk´s_arbejdsprogram` is six tables and eighty-five blank
paragraphs, and it was the document that looked wrong longest. It reads **9
pages** here. That was called an error against the 8 its own hints imply, and
the hints were the thing that was wrong: the file was edited after Word last
drew it. The reader's own export of it is 9 pages, and so is the converter's.

What remains is the one thing this rendering cannot express:

- **Word breaks inside a row.** Four of that document's pages begin partway down
  a row, in the third cell. HTML has nowhere to put a page mark inside a table
  row, so the mark goes on the row — the reader lands at the top of the row
  whose text the page begins in, which is as close as this rendering can be.

Measured against the export page by page, its pages come out between 97 and 102
per cent of theirs.
