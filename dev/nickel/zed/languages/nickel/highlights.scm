; Taken from https://github.com/nickel-lang/tree-sitter-nickel/blob/main/queries/highlights.scm

; MIT License

; Copyright (c) Modus Create LLC and its affiliates

; Permission is hereby granted, free of charge, to any person obtaining a copy
; of this software and associated documentation files (the "Software"), to deal
; in the Software without restriction, including without limitation the rights
; to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
; copies of the Software, and to permit persons to whom the Software is
; furnished to do so, subject to the following conditions:

; The above copyright notice and this permission notice shall be included in all
; copies or substantial portions of the Software.

; THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
; IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
; FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
; AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
; LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
; OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
; SOFTWARE.

(comment) @comment @spell
(annot_atom doc: (static_string) @spell)

[
  "forall"
  "in"
  "let"
  "default"
  "doc"
  "rec"
  "optional"
  "priority"
  "force"
  "not_exported"
] @keyword

"fun" @keyword.function

"import" @include

[ "if" "then" "else" ] @conditional
"match" @conditional

(types) @type
"Array" @type.builtin

; BUILTIN Constants
(bool) @boolean
"null" @constant.builtin
(enum_tag) @constant

(num_literal) @number

(infix_op) @operator

(type_atom) @type

(chunk_literal_single) @string
(chunk_literal_multi) @string

(str_esc_char) @string.escape

[
 "{" "}"
 "(" ")"
 "[|" "|]"
] @punctuation.bracket

[
 ","
 "."
 ":"
 "="
 "|"
 "->"
 "+"
 "-"
 "*"
] @punctuation.delimiter

(multstr_start) @punctuation.bracket
(multstr_end) @punctuation.bracket
(interpolation_start) @punctuation.bracket
(interpolation_end) @punctuation.bracket

(record_field) @field

(builtin) @function.builtin

(fun_expr pats:
  (pattern_fun
    (ident) @parameter
  )
)

; application where the head terms is an identifier: function arg1 arg2 arg3
(applicative t1:
  (applicative (record_operand (atom (ident))) @function)
)

; application where the head terms is a record field path: foo.bar.function arg1 arg2 arg3
(applicative t1:
  (applicative (record_operand (record_operation_chain)) @function)
)
