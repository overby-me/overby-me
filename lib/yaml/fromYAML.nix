# fromYAML — Pure Nix YAML parser.
# Supports block mappings/sequences, flow style, quoted strings,
# block scalars (| and >), and standard YAML scalar types.
lib: let
  h = import ./_helpers.nix lib;

  inherit
    (lib)
    substring
    stringLength
    elemAt
    length
    head
    filter
    genList
    replaceStrings
    isString
    listToAttrs
    concatStringsSep
    ;

  inherit
    (h)
    charAt
    countIndent
    trimRight
    trim
    splitLines
    hasPrefix
    elemOf
    parseQuotedScalar
    isQuoted
    parseScalar
    stripComment
    ;

  # ============================================================
  # Line Preprocessing
  # ============================================================

  preprocessAllLines = text: let
    rawLines = splitLines text;
  in
    lib.imap0 (
      i: line: let
        indent = countIndent line;
        stripped = stripComment line;
        strippedLen = stringLength stripped;
        content =
          if indent >= strippedLen
          then ""
          else trimRight (substring indent (strippedLen - indent) stripped);
      in {
        inherit indent content;
        lineNo = i;
        raw = line;
      }
    )
    rawLines;

  filterMeaningful = allLines:
    filter (l: l.content != "") allLines;

  # ============================================================
  # Content Detection
  # ============================================================

  # Find the position of the mapping colon (': ' or ':' at end) outside quotes.
  # Returns -1 if not a mapping entry.
  findMappingColon = content: let
    len = stringLength content;
    go = i: inSingle: inDouble: bracketDepth:
      if i >= len
      then -1
      else let
        c = charAt content i;
      in
        if inDouble
        then
          if c == "\\"
          then go (i + 2) inSingle inDouble bracketDepth
          else if c == "\""
          then go (i + 1) inSingle false bracketDepth
          else go (i + 1) inSingle inDouble bracketDepth
        else if inSingle
        then
          if c == "'"
          then
            if i + 1 < len && charAt content (i + 1) == "'"
            then go (i + 2) true inDouble bracketDepth
            else go (i + 1) false inDouble bracketDepth
          else go (i + 1) inSingle inDouble bracketDepth
        else if c == "\""
        then go (i + 1) inSingle true bracketDepth
        else if c == "'"
        then go (i + 1) true inDouble bracketDepth
        else if c == "[" || c == "{"
        then go (i + 1) inSingle inDouble (bracketDepth + 1)
        else if c == "]" || c == "}"
        then
          go (i + 1) inSingle inDouble (
            if bracketDepth > 0
            then bracketDepth - 1
            else 0
          )
        else if c == ":" && bracketDepth == 0
        then
          if i + 1 >= len
          then i
          else if charAt content (i + 1) == " " || charAt content (i + 1) == "\t"
          then i
          else go (i + 1) inSingle inDouble bracketDepth
        else go (i + 1) inSingle inDouble bracketDepth;
  in
    go 0 false false 0;

  isMappingLine = content:
  # Don't treat lines starting with special indicators as mapping entries
    !(hasPrefix "- " content || content == "-" || hasPrefix "[" content || hasPrefix "{" content || hasPrefix "? " content || hasPrefix "| " content || hasPrefix "> " content)
    && findMappingColon content >= 0;

  isSequenceLine = content:
    content == "-" || hasPrefix "- " content;

  isBlockScalarIndicator = s:
    elemOf s [
      "|"
      "|-"
      "|+"
      ">"
      ">-"
      ">+"
      "|1"
      "|2"
      "|3"
      "|4"
      "|5"
      "|6"
      "|7"
      "|8"
      "|9"
      "|1-"
      "|2-"
      "|3-"
      "|4-"
      "|5-"
      "|6-"
      "|7-"
      "|8-"
      "|9-"
      "|1+"
      "|2+"
      "|3+"
      "|4+"
      "|5+"
      "|6+"
      "|7+"
      "|8+"
      "|9+"
      ">1"
      ">2"
      ">3"
      ">4"
      ">5"
      ">6"
      ">7"
      ">8"
      ">9"
      ">1-"
      ">2-"
      ">3-"
      ">4-"
      ">5-"
      ">6-"
      ">7-"
      ">8-"
      ">9-"
      ">1+"
      ">2+"
      ">3+"
      ">4+"
      ">5+"
      ">6+"
      ">7+"
      ">8+"
      ">9+"
    ];

  # Split a mapping line into key and value parts
  splitMappingLine = content: let
    colonPos = findMappingColon content;
    rawKey = trim (substring 0 colonPos content);
    key =
      if isQuoted rawKey
      then parseQuotedScalar rawKey
      else rawKey;
    rest = substring (colonPos + 1) (stringLength content - colonPos - 1) content;
    valueStr = trim rest;
  in {
    inherit key valueStr;
  };

  # ============================================================
  # Flow Style Parser (for inline [...] and {...})
  # ============================================================

  skipSpaces = str: pos: let
    len = stringLength str;
    go = i:
      if i >= len
      then i
      else if charAt str i == " " || charAt str i == "\t"
      then go (i + 1)
      else i;
  in
    go pos;

  parseFlowValue = str: startPos: let
    pos = skipSpaces str startPos;
    len = stringLength str;
  in
    if pos >= len
    then {
      value = null;
      inherit pos;
    }
    else let
      c = charAt str pos;
    in
      if c == "["
      then parseFlowSeq str (pos + 1)
      else if c == "{"
      then parseFlowMap str (pos + 1)
      else if c == "\""
      then parseFlowDoubleQuoted str (pos + 1)
      else if c == "'"
      then parseFlowSingleQuoted str (pos + 1)
      else parseFlowPlain str pos;

  parseFlowSeq = str: startPos: let
    pos = skipSpaces str startPos;
    len = stringLength str;
  in
    if pos >= len || charAt str pos == "]"
    then {
      value = [];
      pos =
        if pos < len && charAt str pos == "]"
        then pos + 1
        else pos;
    }
    else let
      go = pos_: items: let
        parsed = parseFlowValue str pos_;
        afterValue = skipSpaces str parsed.pos;
      in
        if afterValue >= len
        then {
          value = items ++ [parsed.value];
          pos = afterValue;
        }
        else if charAt str afterValue == "]"
        then {
          value = items ++ [parsed.value];
          pos = afterValue + 1;
        }
        else if charAt str afterValue == ","
        then let
          nextPos = skipSpaces str (afterValue + 1);
        in
          # Handle trailing comma before ]
          if nextPos < len && charAt str nextPos == "]"
          then {
            value = items ++ [parsed.value];
            pos = nextPos + 1;
          }
          else go nextPos (items ++ [parsed.value])
        else {
          value = items ++ [parsed.value];
          pos = afterValue;
        };
    in
      go pos [];

  parseFlowMap = str: startPos: let
    pos = skipSpaces str startPos;
    len = stringLength str;
  in
    if pos >= len || charAt str pos == "}"
    then {
      value = {};
      pos =
        if pos < len && charAt str pos == "}"
        then pos + 1
        else pos;
    }
    else let
      go = pos_: entries: let
        keyParsed = parseFlowValue str pos_;
        afterKey = skipSpaces str keyParsed.pos;
        # Expect ':' separator
        afterColon =
          if afterKey < len && charAt str afterKey == ":"
          then skipSpaces str (afterKey + 1)
          else afterKey;
        valParsed = parseFlowValue str afterColon;
        afterVal = skipSpaces str valParsed.pos;
        key =
          if isString keyParsed.value
          then keyParsed.value
          else if (keyParsed.value == null)
          then "null"
          else lib.toString keyParsed.value;
        entry = {
          name = key;
          inherit (valParsed) value;
        };
      in
        if afterVal >= len
        then {
          value = listToAttrs (entries ++ [entry]);
          pos = afterVal;
        }
        else if charAt str afterVal == "}"
        then {
          value = listToAttrs (entries ++ [entry]);
          pos = afterVal + 1;
        }
        else if charAt str afterVal == ","
        then let
          nextPos = skipSpaces str (afterVal + 1);
        in
          # Handle trailing comma before }
          if nextPos < len && charAt str nextPos == "}"
          then {
            value = listToAttrs (entries ++ [entry]);
            pos = nextPos + 1;
          }
          else go nextPos (entries ++ [entry])
        else {
          value = listToAttrs (entries ++ [entry]);
          pos = afterVal;
        };
    in
      go pos [];

  parseFlowDoubleQuoted = str: startPos: let
    len = stringLength str;
    go = pos: acc:
      if pos >= len
      then {
        value = acc;
        inherit pos;
      }
      else let
        c = charAt str pos;
      in
        if c == "\""
        then {
          value = acc;
          pos = pos + 1;
        }
        else if c == "\\"
        then let
          next =
            if pos + 1 < len
            then charAt str (pos + 1)
            else "";
          escaped =
            if next == "n"
            then "\n"
            else if next == "t"
            then "\t"
            else if next == "r"
            then "\r"
            else if next == "\\"
            then "\\"
            else if next == "\""
            then "\""
            else if next == "/"
            then "/"
            else if next == "0"
            then ""
            else next;
        in
          go (pos + 2) (acc + escaped)
        else go (pos + 1) (acc + c);
  in
    go startPos "";

  parseFlowSingleQuoted = str: startPos: let
    len = stringLength str;
    go = pos: acc:
      if pos >= len
      then {
        value = acc;
        inherit pos;
      }
      else let
        c = charAt str pos;
      in
        if c == "'"
        then
          if pos + 1 < len && charAt str (pos + 1) == "'"
          then go (pos + 2) (acc + "'")
          else {
            value = acc;
            pos = pos + 1;
          }
        else go (pos + 1) (acc + c);
  in
    go startPos "";

  parseFlowPlain = str: startPos: let
    len = stringLength str;
    go = pos:
      if pos >= len
      then pos
      else let
        c = charAt str pos;
      in
        if c == "," || c == "]" || c == "}"
        then pos
        else if c == ":"
        then
          if pos + 1 < len && (charAt str (pos + 1) == " " || charAt str (pos + 1) == "\t" || charAt str (pos + 1) == "," || charAt str (pos + 1) == "]" || charAt str (pos + 1) == "}")
          then pos
          else if pos + 1 >= len
          then pos
          else go (pos + 1)
        else go (pos + 1);
    endPos = go startPos;
    raw = trim (substring startPos (endPos - startPos) str);
  in {
    value = parseScalar raw;
    pos = endPos;
  };

  # Parse a complete flow value from a content string
  parseFlowValueFull = content: let
    result = parseFlowValue content 0;
  in
    result.value;

  # ============================================================
  # Block Scalar Parser (| and >)
  # ============================================================

  parseBlockScalarContent = allLines: currentLineNo: parentIndent: indicator: let
    isLiteral = hasPrefix "|" indicator;
    chompStr = let
      stripped =
        replaceStrings ["|" ">" "1" "2" "3" "4" "5" "6" "7" "8" "9"] ["" "" "" "" "" "" "" "" "" "" ""] indicator;
    in
      stripped;
    chomp =
      if chompStr == "-"
      then "strip"
      else if chompStr == "+"
      then "keep"
      else "clip";

    # Extract explicit indent from indicator (e.g., |2 means 2 spaces)
    indentDigit = let
      digits = filter (c: elemOf c ["1" "2" "3" "4" "5" "6" "7" "8" "9"]) (
        map (i: charAt indicator i) (genList (i: i) (stringLength indicator))
      );
    in
      if digits == []
      then null
      else lib.fromJSON (head digits);

    # Find lines belonging to this block scalar
    # Starting from the line after the indicator
    startSearchLineNo = currentLineNo + 1;

    # Work directly with the raw text lines
    rawLines = splitLines (concatStringsSep "\n" (map (l: l.raw) allLines));
    totalRawLines = length rawLines;

    collectRawLines = lineNo: acc: scalarIndent:
      if lineNo >= totalRawLines
      then {
        lines = acc;
        nextLineNo = lineNo;
        indent = scalarIndent;
      }
      else let
        rawLine = elemAt rawLines lineNo;
        lineIndent = countIndent rawLine;
        trimmedContent = trim rawLine;
      in
        if trimmedContent == ""
        then
          # Empty lines are part of block scalar
          collectRawLines (lineNo + 1) (acc ++ [""]) scalarIndent
        else if scalarIndent == null
        then
          # First content line determines the indent
          if lineIndent <= parentIndent
          then {
            lines = acc;
            nextLineNo = lineNo;
            indent = scalarIndent;
          }
          else let
            detectedIndent =
              if indentDigit != null
              then parentIndent + indentDigit
              else lineIndent;
          in
            collectRawLines (lineNo + 1) (acc ++ [rawLine]) detectedIndent
        else if lineIndent < scalarIndent
        then {
          lines = acc;
          nextLineNo = lineNo;
          indent = scalarIndent;
        }
        else collectRawLines (lineNo + 1) (acc ++ [rawLine]) scalarIndent;

    collected = collectRawLines startSearchLineNo [] null;

    scalarIndent =
      if collected.indent != null
      then collected.indent
      else parentIndent + 2;

    # Strip the indent from collected lines
    stripIndent = line:
      if trim line == ""
      then ""
      else if stringLength line >= scalarIndent
      then substring scalarIndent (stringLength line - scalarIndent) line
      else line;

    strippedLines = map stripIndent collected.lines;

    # Remove trailing empty lines for chomp processing
    removeTrailingEmpty = lines: let
      go = ls:
        if ls == []
        then []
        else if lib.last ls == ""
        then go (lib.init ls)
        else ls;
    in
      go lines;

    baseLines = removeTrailingEmpty strippedLines;

    # Apply chomping
    chompedContent = let
      base = concatStringsSep "\n" baseLines;
    in
      if chomp == "strip"
      then base
      else if chomp == "keep"
      then
        concatStringsSep "\n" strippedLines
        + (
          if strippedLines != []
          then "\n"
          else ""
        )
      else base + "\n"; # clip: single trailing newline

    # Determine the next meaningful line number
    inherit (collected) nextLineNo;
  in {
    value =
      if isLiteral
      then chompedContent
      else let
        # For folded scalars, fold lines (replace single newlines with spaces)
        foldLines = lines: let
          go = i: acc: currentPara:
            if i >= length lines
            then
              if currentPara == ""
              then acc
              else acc ++ [currentPara]
            else let
              line = elemAt lines i;
            in
              if line == ""
              then go (i + 1) (acc ++ [currentPara] ++ [""]) ""
              else
                go (i + 1) acc (
                  if currentPara == ""
                  then line
                  else currentPara + " " + line
                );
        in
          go 0 [] "";
        folded = foldLines baseLines;
        foldedStr = concatStringsSep "\n" folded;
      in
        if chomp == "strip"
        then foldedStr
        else if chomp == "keep"
        then
          foldedStr
          + (
            if strippedLines != []
            then "\n"
            else ""
          )
        else foldedStr + "\n";
    inherit nextLineNo;
  };

  # ============================================================
  # Block Style Parser
  # ============================================================

  # Parse a value from the meaningful lines array.
  # allLines: all preprocessed lines (including empty ones)
  # lines: meaningful lines only
  # pos: current position in meaningful lines
  # minIndent: minimum indent for this value
  # Returns { value, nextPos }
  parseBlockValue = allLines: lines: pos: minIndent:
    if pos >= length lines
    then {
      value = null;
      nextPos = pos;
    }
    else let
      line = elemAt lines pos;
    in
      if line.indent < minIndent
      then {
        value = null;
        nextPos = pos;
      }
      else let
        inherit (line) content;
      in
        if isSequenceLine content
        then parseBlockSequence allLines lines pos line.indent
        else if hasPrefix "[" content
        then {
          value = parseFlowValueFull content;
          nextPos = pos + 1;
        }
        else if hasPrefix "{" content
        then {
          value = parseFlowValueFull content;
          nextPos = pos + 1;
        }
        else if isBlockScalarIndicator content
        then let
          result = parseBlockScalarContent allLines line.lineNo line.indent content;
          # Find the next meaningful line position
          findNextPos = p:
            if p >= length lines
            then p
            else if (elemAt lines p).lineNo >= result.nextLineNo
            then p
            else findNextPos (p + 1);
        in {
          inherit (result) value;
          nextPos = findNextPos (pos + 1);
        }
        else if isMappingLine content
        then parseBlockMapping allLines lines pos line.indent
        else {
          value = parseScalar content;
          nextPos = pos + 1;
        };

  # Parse a block mapping
  parseBlockMapping = allLines: lines: startPos: indent: let
    go = pos: entries:
      if pos >= length lines
      then {
        value = listToAttrs entries;
        nextPos = pos;
      }
      else let
        line = elemAt lines pos;
      in
        if line.indent != indent || !(isMappingLine line.content)
        then {
          value = listToAttrs entries;
          nextPos = pos;
        }
        else let
          kv = splitMappingLine line.content;
          parsedValue =
            if kv.valueStr == ""
            then
              # Value is on next line(s)
              parseBlockValue allLines lines (pos + 1) (indent + 1)
            else if isBlockScalarIndicator kv.valueStr
            then let
              result = parseBlockScalarContent allLines line.lineNo indent kv.valueStr;
              findNextPos = p:
                if p >= length lines
                then p
                else if (elemAt lines p).lineNo >= result.nextLineNo
                then p
                else findNextPos (p + 1);
            in {
              inherit (result) value;
              nextPos = findNextPos (pos + 1);
            }
            else if hasPrefix "[" kv.valueStr
            then {
              value = parseFlowValueFull kv.valueStr;
              nextPos = pos + 1;
            }
            else if hasPrefix "{" kv.valueStr
            then {
              value = parseFlowValueFull kv.valueStr;
              nextPos = pos + 1;
            }
            else {
              value = parseScalar kv.valueStr;
              nextPos = pos + 1;
            };
          entry = {
            name = kv.key;
            inherit (parsedValue) value;
          };
        in
          go parsedValue.nextPos (entries ++ [entry]);
  in
    go startPos [];

  # Parse a block sequence
  parseBlockSequence = allLines: lines: startPos: indent: let
    go = pos: items:
      if pos >= length lines
      then {
        value = items;
        nextPos = pos;
      }
      else let
        line = elemAt lines pos;
      in
        if line.indent != indent || !(isSequenceLine line.content)
        then {
          value = items;
          nextPos = pos;
        }
        else let
          afterDash =
            if line.content == "-"
            then ""
            else trim (substring 2 (stringLength line.content - 2) line.content);
          parsed = parseDashContent allLines lines pos indent afterDash;
        in
          go parsed.nextPos (items ++ [parsed.value]);
  in
    go startPos [];

  # Parse the content after a sequence dash
  parseDashContent = allLines: lines: pos: indent: afterDash:
    if afterDash == ""
    then
      # Value is on next line(s)
      parseBlockValue allLines lines (pos + 1) (indent + 1)
    else if hasPrefix "[" afterDash
    then {
      value = parseFlowValueFull afterDash;
      nextPos = pos + 1;
    }
    else if hasPrefix "{" afterDash
    then {
      value = parseFlowValueFull afterDash;
      nextPos = pos + 1;
    }
    else if isBlockScalarIndicator afterDash
    then let
      line = elemAt lines pos;
      result = parseBlockScalarContent allLines line.lineNo indent afterDash;
      findNextPos = p:
        if p >= length lines
        then p
        else if (elemAt lines p).lineNo >= result.nextLineNo
        then p
        else findNextPos (p + 1);
    in {
      inherit (result) value;
      nextPos = findNextPos (pos + 1);
    }
    else if isSequenceLine afterDash
    then
      # Nested sequence on the same line: "- - item"
      let
        dashContentIndent = indent + 2;
        innerAfterDash =
          if afterDash == "-"
          then ""
          else trim (substring 2 (stringLength afterDash - 2) afterDash);
        firstItem = parseDashContent allLines lines pos dashContentIndent innerAfterDash;
        # Collect remaining items at dashContentIndent
        restItems = collectMoreSequenceItems allLines lines firstItem.nextPos dashContentIndent;
      in {
        value = [firstItem.value] ++ restItems.items;
        inherit (restItems) nextPos;
      }
    else if isMappingLine afterDash
    then
      # Mapping starts on the dash line
      parseDashMapping allLines lines pos indent afterDash
    else {
      value = parseScalar afterDash;
      nextPos = pos + 1;
    };

  # Collect more sequence items at a given indent level
  collectMoreSequenceItems = allLines: lines: startPos: indent: let
    go = pos: items:
      if pos >= length lines
      then {
        inherit items;
        nextPos = pos;
      }
      else let
        line = elemAt lines pos;
      in
        if line.indent != indent || !(isSequenceLine line.content)
        then {
          inherit items;
          nextPos = pos;
        }
        else let
          afterDash =
            if line.content == "-"
            then ""
            else trim (substring 2 (stringLength line.content - 2) line.content);
          parsed = parseDashContent allLines lines pos indent afterDash;
        in
          go parsed.nextPos (items ++ [parsed.value]);
  in
    go startPos [];

  # Parse a mapping that starts on a dash line (e.g., "- key: value\n  key2: value2")
  parseDashMapping = allLines: lines: pos: indent: afterDash: let
    kv = splitMappingLine afterDash;
    dashContentIndent = indent + 2;
    firstParsedValue =
      if kv.valueStr == ""
      then parseBlockValue allLines lines (pos + 1) (dashContentIndent + 1)
      else if isBlockScalarIndicator kv.valueStr
      then let
        line = elemAt lines pos;
        result = parseBlockScalarContent allLines line.lineNo dashContentIndent kv.valueStr;
        findNextPos = p:
          if p >= length lines
          then p
          else if (elemAt lines p).lineNo >= result.nextLineNo
          then p
          else findNextPos (p + 1);
      in {
        inherit (result) value;
        nextPos = findNextPos (pos + 1);
      }
      else if hasPrefix "[" kv.valueStr
      then {
        value = parseFlowValueFull kv.valueStr;
        nextPos = pos + 1;
      }
      else if hasPrefix "{" kv.valueStr
      then {
        value = parseFlowValueFull kv.valueStr;
        nextPos = pos + 1;
      }
      else {
        value = parseScalar kv.valueStr;
        nextPos = pos + 1;
      };
    firstEntry = {
      name = kv.key;
      inherit (firstParsedValue) value;
    };
    # Collect remaining mapping entries at dashContentIndent
    restResult = collectMappingEntries allLines lines firstParsedValue.nextPos dashContentIndent;
  in {
    value = listToAttrs ([firstEntry] ++ restResult.entries);
    inherit (restResult) nextPos;
  };

  # Collect mapping entries at a specific indent level
  collectMappingEntries = allLines: lines: startPos: indent: let
    go = pos: entries:
      if pos >= length lines
      then {
        inherit entries;
        nextPos = pos;
      }
      else let
        line = elemAt lines pos;
      in
        if line.indent != indent || !(isMappingLine line.content)
        then {
          inherit entries;
          nextPos = pos;
        }
        else let
          kv = splitMappingLine line.content;
          parsedValue =
            if kv.valueStr == ""
            then parseBlockValue allLines lines (pos + 1) (indent + 1)
            else if isBlockScalarIndicator kv.valueStr
            then let
              result = parseBlockScalarContent allLines line.lineNo indent kv.valueStr;
              findNextPos = p:
                if p >= length lines
                then p
                else if (elemAt lines p).lineNo >= result.nextLineNo
                then p
                else findNextPos (p + 1);
            in {
              inherit (result) value;
              nextPos = findNextPos (pos + 1);
            }
            else if hasPrefix "[" kv.valueStr
            then {
              value = parseFlowValueFull kv.valueStr;
              nextPos = pos + 1;
            }
            else if hasPrefix "{" kv.valueStr
            then {
              value = parseFlowValueFull kv.valueStr;
              nextPos = pos + 1;
            }
            else {
              value = parseScalar kv.valueStr;
              nextPos = pos + 1;
            };
          entry = {
            name = kv.key;
            inherit (parsedValue) value;
          };
        in
          go parsedValue.nextPos (entries ++ [entry]);
  in
    go startPos [];
in {
  # ============================================================
  # fromYAML — Main Entry Point
  # ============================================================

  fromYAML = text: let
    allLines = preprocessAllLines text;
    meaningful = filterMeaningful allLines;
    # Filter out document start/end markers (--- / ...)
    filtered = filter (l: l.content != "..." && l.content != "---") meaningful;
  in
    if filtered == []
    then null
    else let
      result = parseBlockValue allLines filtered 0 0;
    in
      result.value;
}
