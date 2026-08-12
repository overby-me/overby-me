// What the app made of a Word document: how many pages its control offers, and
// what each of those pages begins with.
JSON.stringify({
	// Whether the document was rendered at all. A reader who cannot see the file
	// gets no reader, and "no page control" would otherwise read as "one page".
	opened: !!document.querySelector(".docx-doc .docx"),
	control: (() => {
		const c = document.querySelector(".pdf-pages");
		return c ? c.innerText.replace(/\s+/g, " ").trim() : null;
	})(),
	// The breaks BETWEEN pages. The rule at the foot of the last page carries the
	// same class and is not one of them: counted, it added a tenth page to a
	// nine-page document and left the last thing checked pointing at nothing.
	marks: document.querySelectorAll(
		".docx-doc .pdf-page-break:not(.pdf-page-last)",
	).length,
	// What each page begins with. A mark inside a table sits in a row of its own,
	// so "the next sibling" is nothing: this takes the first text AFTER the mark
	// in document order, wherever it lives.
	startsWith: (() => {
		const doc = document.querySelector(".docx-doc .docx");
		if (!doc) return [];
		const flow = [...doc.querySelectorAll(".docx-p, .docx-h, li, td")];
		return [
			...document.querySelectorAll(
				".docx-doc .pdf-page-break:not(.pdf-page-last)",
			),
		].map((m) => {
			const next = flow.find(
				(el) =>
					!el.contains(m) &&
					m.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING &&
					el.innerText.trim().length > 0,
			);
			// Long enough to outrun what is compared against it. Cut to 56, a page
			// whose break was exactly right read as wrong: the expectation it is
			// tested against was 59 characters, and no prefix of a shorter string can
			// start with a longer one.
			return next
				? next.innerText.replace(/\s+/g, " ").trim().slice(0, 160)
				: null;
		});
	})(),
});
