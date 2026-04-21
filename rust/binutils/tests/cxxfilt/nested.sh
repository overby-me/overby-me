# Test c++filt with deeply nested template names
cat > "$TMPDIR/input" << 'EOF'
_ZN5boost6detail15sp_counted_baseD2Ev
_ZSt4moveIRNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEEEEONSt16remove_referenceIT_E4typeEOS8_
_ZNSt6vectorISt4pairIiSt6chrono10time_pointINS2_3_V212system_clockENS2_8durationIlSt5ratioILl1ELl1000000000EEEEEES1_IiS9_ESaISA_EE9push_backEOSA_
EOF
$REF < "$TMPDIR/input" > "$TMPDIR/expected" 2>&1 || true
$RUST < "$TMPDIR/input" > "$TMPDIR/actual" 2>&1 || true
compare "c++filt nested templates"