# Starlark lexer: source text -> token list. builtins only.
#
# Produces Python-style INDENT/DEDENT/NEWLINE tokens (suppressed inside
# (), [], {} and across backslash-newline continuations). Blank lines and
# comment-only lines do not affect indentation. Numbers and strings are
# decoded to their Nix values in the token.
#
# Token shape: { type; value?; line; }. `type` is "NAME", "NUMBER", "STRING",
# "NEWLINE", "INDENT", "DEDENT", "EOF", a keyword ("def", "if", ...), or the
# exact operator/punctuation text ("==", "(", "+", ...).
let
  S = import ./str.nix;
  inherit (S) charAt isDigit isAlpha isAlnum isSpace isHexDigit;
  inherit (builtins) substring stringLength elemAt length;

  keywords = [
    "and"
    "or"
    "not"
    "in"
    "if"
    "elif"
    "else"
    "for"
    "def"
    "return"
    "pass"
    "break"
    "continue"
    "load"
    "lambda"
  ];
  isKeyword = w: builtins.elem w keywords;

  # Multi-char operators, checked longest-first.
  ops2 = [
    "=="
    "!="
    "<="
    ">="
    "+="
    "-="
    "*="
    "/="
    "%="
    "&="
    "|="
    "^="
    "**"
    "//"
    "<<"
    ">>"
    "->"
  ];
  ops1 = "+-*/%()[]{},:.;=<>&|^~";

  isOpen = c: c == "(" || c == "[" || c == "{";
  isClose = c: c == ")" || c == "]" || c == "}";

  # Decode escapes inside a non-raw string body.
  decodeEscapes = body: let
    len = stringLength body;
    go = i: acc:
      if i >= len
      then acc
      else let
        c = charAt body i;
      in
        if c == "\\" && i + 1 < len
        then let
          n = charAt body (i + 1);
          mapped =
            if n == "n"
            then "\n"
            else if n == "t"
            then "\t"
            else if n == "r"
            then "\r"
            else if n == "\\"
            then "\\"
            else if n == "\""
            then "\""
            else if n == "'"
            then "'"
            else if n == "0"
            then "" # NUL is unrepresentable in Nix strings
            else if n == "\n"
            then "" # line continuation inside a string
            else "\\" + n; # leave unknown escapes literal
        in
          go (i + 2) (acc + mapped)
        else go (i + 1) (acc + c);
  in
    go 0 "";

  # Read a string literal. `i` points at the opening quote. `raw` disables
  # escape processing. Returns { value; pos; lines; }.
  readString = src: i: raw: let
    len = stringLength src;
    quote = charAt src i;
    triple = i + 2 < len && charAt src (i + 1) == quote && charAt src (i + 2) == quote;
    qlen =
      if triple
      then 3
      else 1;
    start = i + qlen;
    # Scan to the matching close quote, honoring backslash escapes.
    scan = j:
      if j >= len
      then j # unterminated; take rest
      else let
        c = charAt src j;
      in
        if c == "\\" && !raw && j + 1 < len
        then scan (j + 2)
        else if c == "\\" && raw && j + 1 < len
        then scan (j + 2) # raw: backslash still shields the next char from closing
        else if
          (
            if triple
            then j + 2 < len && charAt src (j + 1) == quote && charAt src (j + 2) == quote
            else true
          )
          && c == quote
          && (triple || true)
        then j
        else scan (j + 1);
    closeAt = scan start;
    body = substring start (closeAt - start) src;
    value =
      if raw
      then body
      else decodeEscapes body;
    afterClose =
      if closeAt >= len
      then len
      else closeAt + qlen;
  in {
    inherit value;
    pos = afterClose;
    lines = countNewlines (substring i (afterClose - i) src);
  };

  countNewlines = s: length (builtins.filter builtins.isList (builtins.split "\n" s));

  # Read a number starting at i. Returns { value; pos; }.
  readNumber = src: i: let
    len = stringLength src;
    two = substring i 2 src;
    radixDigits = base: pred: let
      go = j:
        if j < len && (pred (charAt src j) || charAt src j == "_")
        then go (j + 1)
        else j;
      endp = go (i + 2);
      digits = builtins.replaceStrings ["_"] [""] (substring (i + 2) (endp - (i + 2)) src);
      val = parseRadix base digits;
    in {
      value = val;
      pos = endp;
    };
  in
    if two == "0x" || two == "0X"
    then radixDigits 16 isHexDigit
    else if two == "0o" || two == "0O"
    then radixDigits 8 (c: c >= "0" && c <= "7")
    else if two == "0b" || two == "0B"
    then radixDigits 2 (c: c == "0" || c == "1")
    else let
      # decimal int or float
      digits = j:
        if j < len && (isDigit (charAt src j) || charAt src j == "_")
        then digits (j + 1)
        else j;
      intEnd = digits i;
      hasDot = intEnd < len && charAt src intEnd == ".";
      fracEnd =
        if hasDot
        then digits (intEnd + 1)
        else intEnd;
      hasExp = fracEnd < len && (charAt src fracEnd == "e" || charAt src fracEnd == "E");
      expEnd =
        if hasExp
        then let
          e1 =
            if fracEnd + 1 < len && (charAt src (fracEnd + 1) == "+" || charAt src (fracEnd + 1) == "-")
            then fracEnd + 2
            else fracEnd + 1;
        in
          digits e1
        else fracEnd;
      raw = builtins.replaceStrings ["_"] [""] (substring i (expEnd - i) src);
      value = builtins.fromJSON raw;
    in {
      inherit value;
      pos = expEnd;
    };

  parseRadix = base: digits: let
    len = stringLength digits;
    digitVal = c:
      if c >= "0" && c <= "9"
      then charCodeDigit c
      else if c >= "a" && c <= "f"
      then 10 + (charCodeLower c)
      else if c >= "A" && c <= "F"
      then 10 + (charCodeUpper c)
      else 0;
    go = j: acc:
      if j >= len
      then acc
      else go (j + 1) (acc * base + digitVal (charAt digits j));
  in
    go 0 0;

  # Cheap single-char code helpers for the small alphabets used in radices.
  digitStr = "0123456789";
  lowerStr = "abcdef";
  upperStr = "ABCDEF";
  charCodeDigit = c: S.indexOf c 0 digitStr;
  charCodeLower = c: S.indexOf c 0 lowerStr;
  charCodeUpper = c: S.indexOf c 0 upperStr;

  # Measure indentation width from position i (tabs advance to next mult of 8).
  measureIndent = src: i: let
    len = stringLength src;
    go = j: w:
      if j >= len
      then {
        width = w;
        pos = j;
      }
      else let
        c = charAt src j;
      in
        if c == " "
        then go (j + 1) (w + 1)
        else if c == "\t"
        then go (j + 1) (w - (mod w 8) + 8)
        else {
          width = w;
          pos = j;
        };
  in
    go i 0;

  mod = a: b: a - (b * (a / b));

  matchOp2 = src: i: let
    two = substring i 2 src;
    go = k:
      if k >= length ops2
      then null
      else if elemAt ops2 k == two
      then two
      else go (k + 1);
  in
    if stringLength two < 2
    then null
    else go 0;

  tokenize = src: let
    len = stringLength src;

    # State record threaded through the scan.
    #   pos, line, tokens, indent (stack, list, top last), atLineStart, depth
    step = st:
      if st.pos >= len
      then finalize st
      else if st.atLineStart && st.depth == 0
      then handleLineStart st
      else scanToken st;

    # At the start of a logical line: measure indent, skip blank/comment-only
    # lines, and emit INDENT/DEDENT relative to the stack.
    handleLineStart = st: let
      m = measureIndent src st.pos;
      p = m.pos;
      atEnd = p >= len;
      c =
        if atEnd
        then ""
        else charAt src p;
    in
      if atEnd
      then st // {pos = p;}
      else if c == "\n"
      then
        st
        // {
          pos = p + 1;
          line = st.line + 1;
        } # blank line
      else if c == "#"
      then st // {pos = skipToEol src p;} # comment-only line
      else if c == "\r"
      then st // {pos = p + 1;}
      else let
        top = builtins.head st.indent;
      in
        if m.width > top
        then
          st
          // {
            pos = p;
            atLineStart = false;
            indent = [m.width] ++ st.indent;
            tokens =
              st.tokens
              ++ [
                {
                  type = "INDENT";
                  inherit (st) line;
                }
              ];
          }
        else if m.width < top
        then dedentTo st p m.width
        else
          st
          // {
            pos = p;
            atLineStart = false;
          };

    # Pop indent levels until reaching width; emit a DEDENT per pop.
    dedentTo = st: p: width: let
      go = stk: toks:
        if builtins.head stk > width
        then
          go (builtins.tail stk) (toks
            ++ [
              {
                type = "DEDENT";
                inherit (st) line;
              }
            ])
        else {
          indent = stk;
          tokens = toks;
        };
      r = go st.indent st.tokens;
    in
      st
      // {
        pos = p;
        atLineStart = false;
        inherit (r) indent;
        inherit (r) tokens;
      };

    # Scan a single token in the middle of a line.
    scanToken = st: let
      i = st.pos;
      c = charAt src i;
      next =
        if i + 1 < len
        then charAt src (i + 1)
        else "";
    in
      if isSpace c
      then st // {pos = i + 1;}
      else if c == "\r"
      then st // {pos = i + 1;}
      else if c == "\\" && next == "\n"
      then
        st
        // {
          pos = i + 2;
          line = st.line + 1;
        } # line continuation
      else if c == "#"
      then st // {pos = skipToEol src i;}
      else if c == "\n"
      then emitNewline st
      else if isStringStart src i
      then emitString st
      else if isDigit c || (c == "." && isDigit next)
      then emitNumber st
      else if isAlpha c
      then emitWord st
      else emitOperator st;

    emitNewline = st:
      if st.depth > 0
      then
        st
        // {
          pos = st.pos + 1;
          line = st.line + 1;
        }
      else if lastIsNewlineOrEmpty st
      then
        st
        // {
          pos = st.pos + 1;
          line = st.line + 1;
          atLineStart = true;
        }
      else
        st
        // {
          pos = st.pos + 1;
          line = st.line + 1;
          atLineStart = true;
          tokens =
            st.tokens
            ++ [
              {
                type = "NEWLINE";
                inherit (st) line;
              }
            ];
        };

    # `st.tokens` holds only the CURRENT chunk (see the driver), so an empty one is
    # not necessarily the start of the stream: fall back to the type carried over from
    # the previous chunk, which is null only at the real start.
    lastIsNewlineOrEmpty = st: let
      n = length st.tokens;
    in
      if n == 0
      then st.prevLastType == null || st.prevLastType == "NEWLINE"
      else (elemAt st.tokens (n - 1)).type == "NEWLINE";

    emitString = st: let
      i = st.pos;
      pfx = stringPrefixLen src i;
      raw = stringPrefixIsRaw src i pfx;
      r = readString src (i + pfx) raw;
    in
      st
      // {
        inherit (r) pos;
        line = st.line + r.lines;
        tokens =
          st.tokens
          ++ [
            {
              type = "STRING";
              inherit (r) value;
              inherit (st) line;
            }
          ];
      };

    emitNumber = st: let
      r = readNumber src st.pos;
    in
      st
      // {
        inherit (r) pos;
        tokens =
          st.tokens
          ++ [
            {
              type = "NUMBER";
              inherit (r) value;
              inherit (st) line;
            }
          ];
      };

    emitWord = st: let
      i = st.pos;
      end = let
        go = j:
          if j < len && isAlnum (charAt src j)
          then go (j + 1)
          else j;
      in
        go i;
      word = substring i (end - i) src;
      tok =
        if isKeyword word
        then {
          type = word;
          inherit (st) line;
        }
        else {
          type = "NAME";
          value = word;
          inherit (st) line;
        };
    in
      st
      // {
        pos = end;
        tokens = st.tokens ++ [tok];
      };

    emitOperator = st: let
      i = st.pos;
      c = charAt src i;
      op2 = matchOp2 src i;
    in
      if op2 != null
      then
        st
        // {
          pos = i + 2;
          tokens =
            st.tokens
            ++ [
              {
                type = op2;
                inherit (st) line;
              }
            ];
        }
      else if S.indexOf c 0 ops1 >= 0
      then
        st
        // {
          pos = i + 1;
          depth =
            if isOpen c
            then st.depth + 1
            else if isClose c
            then
              (
                if st.depth > 0
                then st.depth - 1
                else 0
              )
            else st.depth;
          tokens =
            st.tokens
            ++ [
              {
                type = c;
                inherit (st) line;
              }
            ];
        }
      else throw "skylark lexer: unexpected character ${c} at line ${toString st.line}";

    # End of input: close the final logical line, then unwind indentation.
    finalize = st: let
      st1 =
        if !(lastIsNewlineOrEmpty st) && st.depth == 0
        then
          st
          // {
            tokens =
              st.tokens
              ++ [
                {
                  type = "NEWLINE";
                  inherit (st) line;
                }
              ];
          }
        else st;
      dedents = builtins.genList (_: {
        type = "DEDENT";
        inherit (st1) line;
      }) (length st.indent - 1);
    in
      st1
      // {
        pos = len;
        done = true;
        tokens =
          st1.tokens
          ++ dedents
          ++ [
            {
              type = "EOF";
              inherit (st1) line;
            }
          ];
      };

    # Iterate in CHUNKS rather than one self-recursive call per token: Nix has no
    # tail-call elimination, so `loop = st: loop (step st)` costs a C-stack frame per
    # token and overflows on a large file (a generated BUCK file can be tens of
    # thousands of lines). foldl' drives the chunks -- iterative, constant stack --
    # and each chunk recurses only `chunkSize` deep.
    #
    # Every step either advances `pos` or emits one pending DEDENT, so twice the
    # character count plus a margin bounds the step count; running out early is
    # harmless because a finished state is returned unchanged.
    chunkSize = 200;
    stepN = n: st:
      if n == 0 || (st ? done && st.done)
      then st
      else stepN (n - 1) (step st);
    chunkCount = 1 + (2 * len + 16) / chunkSize;

    final = builtins.foldl' (st: _: stepN chunkSize st) {
      pos = 0;
      line = 1;
      tokens = [];
      indent = [0];
      atLineStart = true;
      depth = 0;
      done = false;
    } (builtins.genList (i: i) chunkCount);
  in
    final.tokens;

  skipToEol = src: i: let
    len = stringLength src;
    go = j:
      if j >= len
      then j
      else if charAt src j == "\n"
      then j # leave the newline for emitNewline / lineStart to consume
      else go (j + 1);
  in
    go i;

  # String prefix detection: optional r/b letters immediately before a quote.
  isQuote = c: c == "\"" || c == "'";
  stringPrefixLen = src: i: let
    len = stringLength src;
    c0 =
      if i < len
      then charAt src i
      else "";
    c1 =
      if i + 1 < len
      then charAt src (i + 1)
      else "";
    c2 =
      if i + 2 < len
      then charAt src (i + 2)
      else "";
    isPfx = c: c == "r" || c == "R" || c == "b" || c == "B";
  in
    if isQuote c0
    then 0
    else if isPfx c0 && isQuote c1
    then 1
    else if isPfx c0 && isPfx c1 && isQuote c2
    then 2
    else 0; # not a string
  stringPrefixIsRaw = src: i: pfx: let
    seg = substring i pfx src;
  in
    S.indexOf "r" 0 seg >= 0 || S.indexOf "R" 0 seg >= 0;
  isStringStart = src: i: let
    c = charAt src i;
  in
    isQuote c || (stringPrefixLen src i > 0);
in
  tokenize
