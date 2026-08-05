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
  marks: document.querySelectorAll(".docx-doc .pdf-page-break").length,
  // What each page begins with. A mark inside a table sits in a row of its own,
  // so "the next sibling" is nothing: this takes the first text AFTER the mark
  // in document order, wherever it lives.
  startsWith: (() => {
    const doc = document.querySelector(".docx-doc .docx");
    if (!doc) return [];
    const flow = [...doc.querySelectorAll(".docx-p, .docx-h, li, td")];
    return [...document.querySelectorAll(".docx-doc .pdf-page-break")].map((m) => {
      const next = flow.find((el) =>
        !el.contains(m) &&
        (m.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING) &&
        el.innerText.trim().length > 0
      );
      return next ? next.innerText.replace(/\s+/g, " ").trim().slice(0, 56) : null;
    });
  })(),
});
