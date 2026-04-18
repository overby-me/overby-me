{
  packages = {
    rust-meson = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-meson";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        meta = {
          description = "A Meson build system compatible implementation in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/meson";
          license = lib.licenses.mit;
          mainProgram = "meson";
        };
      };
  };

  checks = let
    # Each entry is { name = "nix-friendly-name"; testDir = "original dir name"; }
    tests = [
      {
        name = "1-trivial";
        testDir = "1 trivial";
      }
      {
        name = "2-cpp";
        testDir = "2 cpp";
      }
      {
        name = "3-static";
        testDir = "3 static";
      }
      {
        name = "4-shared";
        testDir = "4 shared";
      }
      {
        name = "5-linkstatic";
        testDir = "5 linkstatic";
      }
      {
        name = "6-linkshared";
        testDir = "6 linkshared";
      }
      {
        name = "7-mixed";
        testDir = "7 mixed";
      }
      {
        name = "8-install";
        testDir = "8 install";
      }
      {
        name = "9-header-install";
        testDir = "9 header install";
      }
      {
        name = "10-man-install";
        testDir = "10 man install";
      }
      {
        name = "11-subdir";
        testDir = "11 subdir";
      }
      {
        name = "12-data";
        testDir = "12 data";
      }
      {
        name = "13-pch";
        testDir = "13 pch";
      }
      {
        name = "14-configure-file";
        testDir = "14 configure file";
      }
      {
        name = "15-if";
        testDir = "15 if";
      }
      {
        name = "16-comparison";
        testDir = "16 comparison";
      }
      {
        name = "17-array";
        testDir = "17 array";
      }
      {
        name = "18-includedir";
        testDir = "18 includedir";
      }
      {
        name = "18-includedirxyz";
        testDir = "18 includedirxyz";
      }
      {
        name = "19-header-in-file-list";
        testDir = "19 header in file list";
      }
      {
        name = "20-global-arg";
        testDir = "20 global arg";
      }
      {
        name = "21-target-arg";
        testDir = "21 target arg";
      }
      {
        name = "22-object-extraction";
        testDir = "22 object extraction";
      }
      {
        name = "23-endian";
        testDir = "23 endian";
      }
      {
        name = "24-library-versions";
        testDir = "24 library versions";
      }
      {
        name = "25-config-subdir";
        testDir = "25 config subdir";
      }
      {
        name = "26-find-program";
        testDir = "26 find program";
      }
      {
        name = "27-multiline-string";
        testDir = "27 multiline string";
      }
      {
        name = "28-try-compile";
        testDir = "28 try compile";
      }
      {
        name = "29-compiler-id";
        testDir = "29 compiler id";
      }
      {
        name = "30-sizeof";
        testDir = "30 sizeof";
      }
      {
        name = "31-define10";
        testDir = "31 define10";
      }
      {
        name = "32-has-header";
        testDir = "32 has header";
      }
      {
        name = "33-run-program";
        testDir = "33 run program";
      }
      {
        name = "34-logic-ops";
        testDir = "34 logic ops";
      }
      {
        name = "35-string-operations";
        testDir = "35 string operations";
      }
      {
        name = "36-has-function";
        testDir = "36 has function";
      }
      {
        name = "37-has-member";
        testDir = "37 has member";
      }
      {
        name = "38-alignment";
        testDir = "38 alignment";
      }
      {
        name = "39-library-chain";
        testDir = "39 library chain";
      }
      {
        name = "40-options";
        testDir = "40 options";
      }
      {
        name = "41-test-args";
        testDir = "41 test args";
      }
      {
        name = "42-subproject";
        testDir = "42 subproject";
      }
      {
        name = "43-subproject-options";
        testDir = "43 subproject options";
      }
      {
        name = "44-pkgconfig-gen";
        testDir = "44 pkgconfig-gen";
      }
      {
        name = "45-custom-install-dirs";
        testDir = "45 custom install dirs";
      }
      {
        name = "46-subproject-subproject";
        testDir = "46 subproject subproject";
      }
      {
        name = "47-same-file-name";
        testDir = "47 same file name";
      }
      {
        name = "48-file-grabber";
        testDir = "48 file grabber";
      }
      {
        name = "49-custom-target";
        testDir = "49 custom target";
      }
      {
        name = "50-custom-target-chain";
        testDir = "50 custom target chain";
      }
      {
        name = "51-run-target";
        testDir = "51 run target";
      }
      {
        name = "52-object-generator";
        testDir = "52 object generator";
      }
      {
        name = "53-install-script";
        testDir = "53 install script";
      }
      {
        name = "54-custom-target-source-output";
        testDir = "54 custom target source output";
      }
      {
        name = "55-exe-static-shared";
        testDir = "55 exe static shared";
      }
      {
        name = "56-array-methods";
        testDir = "56 array methods";
      }
      {
        name = "57-custom-header-generator";
        testDir = "57 custom header generator";
      }
      {
        name = "58-multiple-generators";
        testDir = "58 multiple generators";
      }
      {
        name = "59-install-subdir";
        testDir = "59 install subdir";
      }
      {
        name = "60-foreach";
        testDir = "60 foreach";
      }
      {
        name = "61-number-arithmetic";
        testDir = "61 number arithmetic";
      }
      {
        name = "62-string-arithmetic";
        testDir = "62 string arithmetic";
      }
      {
        name = "63-array-arithmetic";
        testDir = "63 array arithmetic";
      }
      {
        name = "64-arithmetic-bidmas";
        testDir = "64 arithmetic bidmas";
      }
      {
        name = "65-build-always";
        testDir = "65 build always";
      }
      {
        name = "66-vcstag";
        testDir = "66 vcstag";
      }
      {
        name = "67-modules";
        testDir = "67 modules";
      }
      {
        name = "68-should-fail";
        testDir = "68 should fail";
      }
      {
        name = "69-configure-file-in-custom-target";
        testDir = "69 configure file in custom target";
      }
      {
        name = "70-external-test-program";
        testDir = "70 external test program";
      }
      {
        name = "71-ctarget-dependency";
        testDir = "71 ctarget dependency";
      }
      {
        name = "72-shared-subproject";
        testDir = "72 shared subproject";
      }
      {
        name = "73-shared-subproject-2";
        testDir = "73 shared subproject 2";
      }
      {
        name = "74-file-object";
        testDir = "74 file object";
      }
      {
        name = "75-custom-subproject-dir";
        testDir = "75 custom subproject dir";
      }
      {
        name = "76-has-type";
        testDir = "76 has type";
      }
      {
        name = "77-extract-from-nested-subdir";
        testDir = "77 extract from nested subdir";
      }
      {
        name = "78-internal-dependency";
        testDir = "78 internal dependency";
      }
      {
        name = "79-same-basename";
        testDir = "79 same basename";
      }
      {
        name = "80-declare-dep";
        testDir = "80 declare dep";
      }
      {
        name = "81-extract-all";
        testDir = "81 extract all";
      }
      {
        name = "82-add-language";
        testDir = "82 add language";
      }
      {
        name = "83-identical-target-name-in-subproject";
        testDir = "83 identical target name in subproject";
      }
      {
        name = "84-plusassign";
        testDir = "84 plusassign";
      }
      {
        name = "85-skip-subdir";
        testDir = "85 skip subdir";
      }
      {
        name = "86-private-include";
        testDir = "86 private include";
      }
      {
        name = "87-default-options";
        testDir = "87 default options";
      }
      {
        name = "88-dep-fallback";
        testDir = "88 dep fallback";
      }
      {
        name = "89-default-library";
        testDir = "89 default library";
      }
      {
        name = "90-gen-extra";
        testDir = "90 gen extra";
      }
      {
        name = "91-benchmark";
        testDir = "91 benchmark";
      }
      {
        name = "92-test-workdir";
        testDir = "92 test workdir";
      }
      {
        name = "93-suites";
        testDir = "93 suites";
      }
      {
        name = "94-threads";
        testDir = "94 threads";
      }
      {
        name = "95-manygen";
        testDir = "95 manygen";
      }
      {
        name = "96-stringdef";
        testDir = "96 stringdef";
      }
      {
        name = "97-find-program-path";
        testDir = "97 find program path";
      }
      {
        name = "98-subproject-subdir";
        testDir = "98 subproject subdir";
      }
      {
        name = "99-postconf";
        testDir = "99 postconf";
      }
      {
        name = "100-postconf-with-args";
        testDir = "100 postconf with args";
      }
      {
        name = "101-testframework-options";
        testDir = "101 testframework options";
      }
      {
        name = "102-extract-same-name";
        testDir = "102 extract same name";
      }
      {
        name = "103-has-header-symbol";
        testDir = "103 has header symbol";
      }
      {
        name = "104-has-arg";
        testDir = "104 has arg";
      }
      {
        name = "105-generatorcustom";
        testDir = "105 generatorcustom";
      }
      {
        name = "106-multiple-dir-configure-file";
        testDir = "106 multiple dir configure file";
      }
      {
        name = "107-spaces-backslash";
        testDir = "107 spaces backslash";
      }
      {
        name = "108-ternary";
        testDir = "108 ternary";
      }
      {
        name = "109-custom-target-capture";
        testDir = "109 custom target capture";
      }
      {
        name = "110-allgenerate";
        testDir = "110 allgenerate";
      }
      {
        name = "111-pathjoin";
        testDir = "111 pathjoin";
      }
      {
        name = "112-subdir-subproject";
        testDir = "112 subdir subproject";
      }
      {
        name = "113-interpreter-copy-mutable-var-on-assignment";
        testDir = "113 interpreter copy mutable var on assignment";
      }
      {
        name = "114-skip";
        testDir = "114 skip";
      }
      {
        name = "115-subproject-project-arguments";
        testDir = "115 subproject project arguments";
      }
      {
        name = "116-test-skip";
        testDir = "116 test skip";
      }
      {
        name = "117-shared-module";
        testDir = "117 shared module";
      }
      {
        name = "118-llvm-ir-and-assembly";
        testDir = "118 llvm ir and assembly";
      }
      {
        name = "119-cpp-and-asm";
        testDir = "119 cpp and asm";
      }
      {
        name = "120-extract-all-shared-library";
        testDir = "120 extract all shared library";
      }
      {
        name = "121-object-only-target";
        testDir = "121 object only target";
      }
      {
        name = "122-no-buildincdir";
        testDir = "122 no buildincdir";
      }
      {
        name = "123-custom-target-directory-install";
        testDir = "123 custom target directory install";
      }
      {
        name = "124-dependency-file-generation";
        testDir = "124 dependency file generation";
      }
      {
        name = "125-configure-file-in-generator";
        testDir = "125 configure file in generator";
      }
      {
        name = "126-generated-llvm-ir";
        testDir = "126 generated llvm ir";
      }
      {
        name = "127-generated-assembly";
        testDir = "127 generated assembly";
      }
      {
        name = "128-build-by-default-targets-in-tests";
        testDir = "128 build by default targets in tests";
      }
      {
        name = "129-build-by-default";
        testDir = "129 build by default";
      }
      {
        name = "130-include-order";
        testDir = "130 include order";
      }
      {
        name = "131-override-options";
        testDir = "131 override options";
      }
      {
        name = "132-get-define";
        testDir = "132 get define";
      }
      {
        name = "133-c-cpp-and-asm";
        testDir = "133 c cpp and asm";
      }
      {
        name = "134-compute-int";
        testDir = "134 compute int";
      }
      {
        name = "135-custom-target-object-output";
        testDir = "135 custom target object output";
      }
      {
        name = "136-empty-build-file";
        testDir = "136 empty build file";
      }
      {
        name = "137-whole-archive";
        testDir = "137 whole archive";
      }
      {
        name = "138-c-and-cpp-link";
        testDir = "138 C and CPP link";
      }
      {
        name = "139-mesonintrospect-from-scripts";
        testDir = "139 mesonintrospect from scripts";
      }
      {
        name = "140-custom-target-multiple-outputs";
        testDir = "140 custom target multiple outputs";
      }
      {
        name = "141-special-characters";
        testDir = "141 special characters";
      }
      {
        name = "142-nested-links";
        testDir = "142 nested links";
      }
      {
        name = "143-list-of-file-sources";
        testDir = "143 list of file sources";
      }
      {
        name = "144-link-depends-custom-target";
        testDir = "144 link depends custom target";
      }
      {
        name = "145-recursive-linking";
        testDir = "145 recursive linking";
      }
      {
        name = "146-library-at-root";
        testDir = "146 library at root";
      }
      {
        name = "147-simd";
        testDir = "147 simd";
      }
      {
        name = "148-shared-module-resolving-symbol-in-executable";
        testDir = "148 shared module resolving symbol in executable";
      }
      {
        name = "149-dotinclude";
        testDir = "149 dotinclude";
      }
      {
        name = "150-reserved-targets";
        testDir = "150 reserved targets";
      }
      {
        name = "151-duplicate-source-names";
        testDir = "151 duplicate source names";
      }
      {
        name = "152-index-customtarget";
        testDir = "152 index customtarget";
      }
      {
        name = "153-wrap-file-should-not-failed";
        testDir = "153 wrap file should not failed";
      }
      {
        name = "154-includedir-subproj";
        testDir = "154 includedir subproj";
      }
      {
        name = "155-subproject-dir-name-collision";
        testDir = "155 subproject dir name collision";
      }
      {
        name = "156-config-tool-variable";
        testDir = "156 config tool variable";
      }
      {
        name = "157-custom-target-subdir-depend-files";
        testDir = "157 custom target subdir depend files";
      }
      {
        name = "158-disabler";
        testDir = "158 disabler";
      }
      {
        name = "159-array-option";
        testDir = "159 array option";
      }
      {
        name = "160-custom-target-template-substitution";
        testDir = "160 custom target template substitution";
      }
      {
        name = "161-not-found-dependency";
        testDir = "161 not-found dependency";
      }
      {
        name = "162-subdir-if_found";
        testDir = "162 subdir if_found";
      }
      {
        name = "163-default-options-prefix-dependent-defaults";
        testDir = "163 default options prefix dependent defaults";
      }
      {
        name = "164-dependency-factory";
        testDir = "164 dependency factory";
      }
      {
        name = "165-get-project-license";
        testDir = "165 get project license";
      }
      {
        name = "166-yield";
        testDir = "166 yield";
      }
      {
        name = "167-subproject-nested-subproject-dirs";
        testDir = "167 subproject nested subproject dirs";
      }
      {
        name = "168-preserve-gendir";
        testDir = "168 preserve gendir";
      }
      {
        name = "169-source-in-dep";
        testDir = "169 source in dep";
      }
      {
        name = "170-generator-link-whole";
        testDir = "170 generator link whole";
      }
      {
        name = "171-initial-c_args";
        testDir = "171 initial c_args";
      }
      {
        name = "172-identical-target-name-in-subproject-flat-layout";
        testDir = "172 identical target name in subproject flat layout";
      }
      {
        name = "173-as-needed";
        testDir = "173 as-needed";
      }
      {
        name = "174-ndebug-if-release-enabled";
        testDir = "174 ndebug if-release enabled";
      }
      {
        name = "175-ndebug-if-release-disabled";
        testDir = "175 ndebug if-release disabled";
      }
      {
        name = "176-subproject-version";
        testDir = "176 subproject version";
      }
      {
        name = "177-subdir_done";
        testDir = "177 subdir_done";
      }
      {
        name = "178-bothlibraries";
        testDir = "178 bothlibraries";
      }
      {
        name = "179-escape-and-unicode";
        testDir = "179 escape and unicode";
      }
      {
        name = "180-has-link-arg";
        testDir = "180 has link arg";
      }
      {
        name = "181-same-target-name-flat-layout";
        testDir = "181 same target name flat layout";
      }
      {
        name = "182-find-override";
        testDir = "182 find override";
      }
      {
        name = "183-partial-dependency";
        testDir = "183 partial dependency";
      }
      {
        name = "184-openmp";
        testDir = "184 openmp";
      }
      {
        name = "185-same-target-name";
        testDir = "185 same target name";
      }
      {
        name = "186-test-depends";
        testDir = "186 test depends";
      }
      {
        name = "187-args-flattening";
        testDir = "187 args flattening";
      }
      {
        name = "188-dict";
        testDir = "188 dict";
      }
      {
        name = "189-check-header";
        testDir = "189 check header";
      }
      {
        name = "190-install_mode";
        testDir = "190 install_mode";
      }
      {
        name = "191-subproject-array-version";
        testDir = "191 subproject array version";
      }
      {
        name = "192-feature-option";
        testDir = "192 feature option";
      }
      {
        name = "193-feature-option-disabled";
        testDir = "193 feature option disabled";
      }
      {
        name = "194-static-threads";
        testDir = "194 static threads";
      }
      {
        name = "195-generator-in-subdir";
        testDir = "195 generator in subdir";
      }
      {
        name = "196-subproject-with-features";
        testDir = "196 subproject with features";
      }
      {
        name = "197-function-attributes";
        testDir = "197 function attributes";
      }
      {
        name = "198-broken-subproject";
        testDir = "198 broken subproject";
      }
      {
        name = "199-argument-syntax";
        testDir = "199 argument syntax";
      }
      {
        name = "200-install-name_prefix-name_suffix";
        testDir = "200 install name_prefix name_suffix";
      }
      {
        name = "201-kwarg-entry";
        testDir = "201 kwarg entry";
      }
      {
        name = "202-custom-target-build-by-default";
        testDir = "202 custom target build by default";
      }
      {
        name = "203-find_library-and-headers";
        testDir = "203 find_library and headers";
      }
      {
        name = "204-line-continuation";
        testDir = "204 line continuation";
      }
      {
        name = "205-native-file-path-override";
        testDir = "205 native file path override";
      }
      {
        name = "206-tap-tests";
        testDir = "206 tap tests";
      }
      {
        name = "207-warning-level-0";
        testDir = "207 warning level 0";
      }
      {
        name = "208-link-custom";
        testDir = "208 link custom";
      }
      {
        name = "209-link-custom_i-single-from-multiple";
        testDir = "209 link custom_i single from multiple";
      }
      {
        name = "210-link-custom_i-multiple-from-multiple";
        testDir = "210 link custom_i multiple from multiple";
      }
      {
        name = "211-dependency-get_variable-method";
        testDir = "211 dependency get_variable method";
      }
      {
        name = "212-source-set-configuration_data";
        testDir = "212 source set configuration_data";
      }
      {
        name = "213-source-set-dictionary";
        testDir = "213 source set dictionary";
      }
      {
        name = "214-source-set-custom-target";
        testDir = "214 source set custom target";
      }
      {
        name = "215-source-set-realistic-example";
        testDir = "215 source set realistic example";
      }
      {
        name = "216-custom-target-input-extracted-objects";
        testDir = "216 custom target input extracted objects";
      }
      {
        name = "217-test-priorities";
        testDir = "217 test priorities";
      }
      {
        name = "218-include_dir-dot";
        testDir = "218 include_dir dot";
      }
      {
        name = "219-include_type-dependency";
        testDir = "219 include_type dependency";
      }
      {
        name = "220-fs-module";
        testDir = "220 fs module";
      }
      {
        name = "221-zlib";
        testDir = "221 zlib";
      }
      {
        name = "222-native-prop";
        testDir = "222 native prop";
      }
      {
        name = "223-persubproject-options";
        testDir = "223 persubproject options";
      }
      {
        name = "224-arithmetic-operators";
        testDir = "224 arithmetic operators";
      }
      {
        name = "225-link-language";
        testDir = "225 link language";
      }
      {
        name = "226-link-depends-indexed-custom-target";
        testDir = "226 link depends indexed custom target";
      }
      {
        name = "227-very-long-command-line";
        testDir = "227 very long command line";
      }
      {
        name = "228-custom_target-source";
        testDir = "228 custom_target source";
      }
      {
        name = "229-disabler-array-addition";
        testDir = "229 disabler array addition";
      }
      {
        name = "230-external-project";
        testDir = "230 external project";
      }
      {
        name = "231-subdir-files";
        testDir = "231 subdir files";
      }
      {
        name = "232-dependency-allow_fallback";
        testDir = "232 dependency allow_fallback";
      }
      {
        name = "233-wrap-case";
        testDir = "233 wrap case";
      }
      {
        name = "234-get_file_contents";
        testDir = "234 get_file_contents";
      }
      {
        name = "235-invalid-standard-overridden-to-valid";
        testDir = "235 invalid standard overridden to valid";
      }
      {
        name = "236-proper-args-splitting";
        testDir = "236 proper args splitting";
      }
      {
        name = "237-fstrings";
        testDir = "237 fstrings";
      }
      {
        name = "238-dependency-include_type-inconsistency";
        testDir = "238 dependency include_type inconsistency";
      }
      {
        name = "239-includedir-violation";
        testDir = "239 includedir violation";
      }
      {
        name = "240-dependency-native-host-==-build";
        testDir = "240 dependency native host == build";
      }
      {
        name = "241-set-and-get-variable";
        testDir = "241 set and get variable";
      }
      {
        name = "242-custom-target-feed";
        testDir = "242 custom target feed";
      }
      {
        name = "243-escapepp";
        testDir = "243 escape++";
      }
      {
        name = "244-variable-scope";
        testDir = "244 variable scope";
      }
      {
        name = "245-custom-target-index-source";
        testDir = "245 custom target index source";
      }
      {
        name = "246-dependency-fallbacks";
        testDir = "246 dependency fallbacks";
      }
      {
        name = "247-deprecated-option";
        testDir = "247 deprecated option";
      }
      {
        name = "248-install_emptydir";
        testDir = "248 install_emptydir";
      }
      {
        name = "249-install_symlink";
        testDir = "249 install_symlink";
      }
      {
        name = "250-system-include-dir";
        testDir = "250 system include dir";
      }
      {
        name = "251-add_project_dependencies";
        testDir = "251 add_project_dependencies";
      }
      {
        name = "252-install-data-structured";
        testDir = "252 install data structured";
      }
      {
        name = "253-subproject-dependency-variables";
        testDir = "253 subproject dependency variables";
      }
      {
        name = "254-long-output";
        testDir = "254 long output";
      }
      {
        name = "255-module-warnings";
        testDir = "255 module warnings";
      }
      {
        name = "256-subproject-extracted-objects";
        testDir = "256 subproject extracted objects";
      }
      {
        name = "257-generated-header-dep";
        testDir = "257 generated header dep";
      }
      {
        name = "258-subsubproject-inplace";
        testDir = "258 subsubproject inplace";
      }
      {
        name = "259-preprocess";
        testDir = "259 preprocess";
      }
      {
        name = "260-declare_dependency-objects";
        testDir = "260 declare_dependency objects";
      }
      {
        name = "261-testcase-clause";
        testDir = "261 testcase clause";
      }
      {
        name = "262-generator-chain";
        testDir = "262 generator chain";
      }
      {
        name = "263-internal-dependency-includes-in-checks";
        testDir = "263 internal dependency includes in checks";
      }
      {
        name = "264-required-keyword-in-has-functions";
        testDir = "264 required keyword in has functions";
      }
      {
        name = "265-default_options-dict";
        testDir = "265 default_options dict";
      }
      {
        name = "266-format-string";
        testDir = "266 format string";
      }
      {
        name = "267-default_options-in-find_program";
        testDir = "267 default_options in find_program";
      }
      {
        name = "268-install-functions-and-follow-symlinks";
        testDir = "268 install functions and follow symlinks";
      }
      {
        name = "269-configure-file-output-format";
        testDir = "269 configure file output format";
      }
      {
        name = "270-int_to_str_fill";
        testDir = "270 int_to_str_fill";
      }
      {
        name = "271-env-in-generator.process";
        testDir = "271 env in generator.process";
      }
      {
        name = "272-unity";
        testDir = "272 unity";
      }
      {
        name = "273-both-libraries";
        testDir = "273 both libraries";
      }
      {
        name = "274-customtarget-exe-for-test";
        testDir = "274 customtarget exe for test";
      }
      {
        name = "275-environment";
        testDir = "275 environment";
      }
      {
        name = "276-required-keyword-in-compiles-functions";
        testDir = "276 required keyword in compiles functions";
      }
      {
        name = "277-generator-custom_tgt-subdir";
        testDir = "277 generator custom_tgt subdir";
      }
      {
        name = "278-custom-target-private-dir";
        testDir = "278 custom target private dir";
      }
      {
        name = "279-pkgconfig-override";
        testDir = "279 pkgconfig override";
      }
      {
        name = "280-pkgconfig-gen";
        testDir = "280 pkgconfig-gen";
      }
      {
        name = "281-subproj-options";
        testDir = "281 subproj options";
      }
      {
        name = "282-test-args-and-depends-in-path";
        testDir = "282 test args and depends in path";
      }
      {
        name = "283-wrap-override";
        testDir = "283 wrap override";
      }
      {
        name = "284-pkgconfig-subproject";
        testDir = "284 pkgconfig subproject";
      }
    ];
  in
    builtins.listToAttrs (map (t: {
        name = "rust-meson-test-${t.name}";
        value = pkgs:
          import ./testsuite.nix {
            inherit pkgs;
            inherit (t) name testDir;
          };
      })
      tests)
    // {
      rust-meson-hello-world = pkgs:
        import ./hello-world-test.nix {inherit pkgs;};
    };
}
