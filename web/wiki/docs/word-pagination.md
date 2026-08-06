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
- **Table columns.** The widths the document states, and a table wider than the
  text column overflows the margin as it does in Word rather than being squeezed
  into it. Left to the browser, one table was laid out 134/60/398 where Word has
  290/75/305, and every row came out short.
- **Pages are filled, not sliced.** Word moves a paragraph whole rather than
  stranding a line of it, so a page can end early and the space below it goes
  unused. Measuring the ribbon continuously and dividing by the page height gets
  this wrong in both directions.
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

## What is checked

`scripts/check-word-pagination.nu` opens five real documents in the deployed
build and compares both halves of the answer — how many pages, and what each
page begins with — against what Word recorded in the files themselves
(`lastRenderedPageBreak`, the note Word leaves where it last drew a break).

Four of the five match Word exactly, including both of the assembly's own. So
does `indstillinger` under Ekstraordinært landsmøde, which is checked by hand
(the test account is not a member of that context).

## The one that does not, and why

`evaluering_af_fu_og_posk´s_arbejdsprogram` is six tables and eighty-five blank
paragraphs. It measures 9 pages where Word makes 8, and four of its breaks are
Word's. Every page it produces is packed to the full 896px, and the document
measures 8.26 pages of ink — so no better filling can make it 8: about 230px of
its content is taller here than in Word.

It read 8 before the blank-paragraph fix below, and that was luck: blank
paragraphs measured half a line, which cancelled the excess. Two known
structural limits remain in this document, both of them about tables:

1. **Word breaks inside a row.** Four of that document's pages begin partway
   down a row, in the third cell. HTML has nowhere to put a page mark inside a
   table row, so the mark goes on the row — the reader lands at the top of the
   row whose text the page begins in, which is as close as this rendering can
   be.
2. **Something in its tables measures too tall**, by about a quarter of a page
   over six tables. Its fifty-three in-cell blank paragraphs are the obvious
   suspect (Word gives an empty paragraph a line, and so does this now), but
   that is a guess until it is measured cell by cell.

LibreOffice was tried as a second opinion and is not one: converting the same
file to PDF gives **ten** pages, against Word's eight and this app's eight. Its
page 8 begins where this app's page 7 does. A different layout engine agrees
with neither, so settling those last three breaks needs the document opened in
Word itself.
