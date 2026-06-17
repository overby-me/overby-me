# toYAML — Pure Nix YAML serializer.
# Converts Nix values to idiomatic block-style YAML strings.
# Supports mappings, sequences, scalars, multi-line block scalars, and proper quoting.
lib: let
  h = import ./_helpers.nix lib;

  inherit
    (lib)
    substring
    stringLength
    head
    tail
    genList
    isString
    isList
    isAttrs
    isBool
    isInt
    isFloat
    attrNames
    typeOf
    concatStringsSep
    ;

  inherit
    (h)
    splitLines
    hasInfix
    hasSuffix
    boolTrueValues
    boolFalseValues
    elemOf
    tryParseNumber
    escapeDoubleQuoted
    isScalar
    ;

  # ============================================================
  # Indentation
  # ============================================================

  ind = depth: concatStringsSep "" (genList (_: "  ") depth);

  # ============================================================
  # String Quoting
  # ============================================================

  # Check if a string needs quoting in YAML
  needsQuoting = s: let
    len = stringLength s;
    first =
      if len > 0
      then substring 0 1 s
      else "";
    last =
      if len > 0
      then substring (len - 1) 1 s
      else "";
  in
    len
    == 0
    || elemOf first ["{" "[" "&" "*" "?" "|" "-" "<" ">" "=" "!" "%" "@" "`" "'" "\"" "," "#" "~" " " "\t"]
    || last == " "
    || last == "\t"
    || last == ":"
    || hasInfix ": " s
    || hasInfix " #" s
    || hasInfix "\n" s
    || hasInfix "\t" s
    || elemOf s boolTrueValues
    || elemOf s boolFalseValues
    || elemOf s ["null" "Null" "NULL" "~"]
    || (tryParseNumber s).success or false
    || s == "---"
    || s == "...";

  # Check if a key needs quoting
  quoteKey = name:
    if needsQuoting name
    then "\"${escapeDoubleQuoted name}\""
    else name;

  # ============================================================
  # Scalar Serialization
  # ============================================================

  # Serialize a scalar value to its YAML string representation
  scalarToString = val:
    if (val == null)
    then "null"
    else if isBool val
    then
      if val
      then "true"
      else "false"
    else if isInt val
    then lib.toString val
    else if isFloat val
    then let
      s = lib.toString val;
    in
      # Ensure floats have a decimal point
      if hasInfix "." s || hasInfix "e" s || hasInfix "E" s
      then s
      else s + ".0"
    else if isString val
    then
      if hasInfix "\n" val
      then
        # Will be rendered as block scalar; this function is only for inline use
        "\"${escapeDoubleQuoted val}\""
      else if needsQuoting val
      then "\"${escapeDoubleQuoted val}\""
      else val
    else throw "toYAML: unsupported scalar type: ${typeOf val}";

  # ============================================================
  # Multi-line String Handling
  # ============================================================

  isMultilineString = val:
    isString val && hasInfix "\n" val;

  # Render a multi-line string as a YAML block scalar at the given depth
  renderBlockScalar = depth: s: let
    # Determine chomp indicator
    endsWithNewline = hasSuffix "\n" s;
    # Strip trailing newline for processing
    content =
      if endsWithNewline
      then substring 0 (stringLength s - 1) s
      else s;
    contentLines = splitLines content;
    indicator =
      if endsWithNewline
      then "|"
      else "|-";
    indented = map (l:
      if l == ""
      then ""
      else "${ind depth}${l}")
    contentLines;
  in "${indicator}\n${concatStringsSep "\n" indented}";

  # ============================================================
  # Block Rendering
  # ============================================================

  # Render a value as a YAML block at the given depth
  renderBlock = depth: val:
    if isScalar val
    then "${ind depth}${scalarToString val}"
    else if isList val
    then
      if val == []
      then "${ind depth}[]"
      else concatStringsSep "\n" (map (renderListItem depth) val)
    else if isAttrs val
    then
      if val == {}
      then "${ind depth}{}"
      else concatStringsSep "\n" (map (renderMappingEntry depth val) (attrNames val))
    else throw "toYAML: unsupported type: ${typeOf val}";

  renderListItem = depth: item:
    if isMultilineString item
    then "${ind depth}- ${renderBlockScalar (depth + 1) item}"
    else if isScalar item
    then "${ind depth}- ${scalarToString item}"
    else if isAttrs item && item != {}
    then let
      keys = attrNames item;
      firstKey = head keys;
      restKeys = tail keys;
      firstVal = item.${firstKey};
      firstRendered = renderInlineOrSub (depth + 1) firstVal;
      firstLine = "${ind depth}- ${quoteKey firstKey}:${firstRendered}";
      restLines = map (key: renderMappingEntry (depth + 1) item key) restKeys;
    in
      concatStringsSep "\n" ([firstLine] ++ restLines)
    else if isList item && item != []
    then "${ind depth}-\n${renderBlock (depth + 1) item}"
    else
      # Empty list or empty attrs
      "${ind depth}- ${
        if isList item
        then "[]"
        else "{}"
      }";

  renderMappingEntry = depth: attrs: key: let
    val = attrs.${key};
    rendered = renderInlineOrSub (depth + 1) val;
  in "${ind depth}${quoteKey key}:${rendered}";

  # Returns either " value" for scalars or "\n<block>" for collections
  renderInlineOrSub = depth: val:
    if isMultilineString val
    then " ${renderBlockScalar depth val}"
    else if isScalar val
    then " ${scalarToString val}"
    else if isList val
    then
      if val == []
      then " []"
      else "\n${renderBlock depth val}"
    else if isAttrs val
    then
      if val == {}
      then " {}"
      else "\n${renderBlock depth val}"
    else throw "toYAML: unsupported type: ${typeOf val}";
in {
  # ============================================================
  # toYAML — Main Entry Point
  # ============================================================

  toYAML = val:
    if (val == null)
    then "null\n"
    else if isScalar val
    then "${scalarToString val}\n"
    else "${renderBlock 0 val}\n";
}
