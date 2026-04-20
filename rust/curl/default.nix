{
  packages = {
    rust-curl = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-curl";
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
          description = "A curl-compatible HTTP client written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/curl";
          license = lib.licenses.mit;
          mainProgram = "curl";
        };
      };

    rust-curl-dev = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-curl-dev";
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

        buildType = "debug";

        meta = {
          description = "A curl-compatible HTTP client written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/curl";
          license = lib.licenses.mit;
          mainProgram = "curl";
        };
      };
  };

  # Discovery tool: runs tests 1-100 and reports which pass/fail
  # Usage: nix build .#packages.x86_64-linux.rust-curl-test-discovery -L
  packages.rust-curl-test-discovery = {
    lib,
    perl,
    coreutils,
    diffutils,
    gnused,
    gnugrep,
    stunnel,
    rust-curl-dev,
    curl,
    stdenv,
    autoreconfHook,
    pkg-config,
    python3,
    openssl,
    zlib,
    nghttp2,
    libpsl,
  }: let
    curl-test-infra = stdenv.mkDerivation {
      pname = "curl-test-infra";
      inherit (curl) version src;
      nativeBuildInputs = [autoreconfHook pkg-config perl python3];
      buildInputs = [openssl zlib nghttp2 libpsl];
      postPatch = ''
        patchShebangs scripts/
      '';
      configureFlags = [
        "--with-openssl"
        "--without-libssh2"
        "--disable-ldap"
        "--without-brotli"
        "--without-zstd"
        "--without-librtmp"
        "--without-libidn2"
        "--disable-docs"
      ];
      buildPhase = ''
        make -C lib -j$NIX_BUILD_CORES
        make -C src -j$NIX_BUILD_CORES
        make -C tests -j$NIX_BUILD_CORES
      '';
      installPhase = ''
        mkdir -p $out/lib $out/src
        cp -r tests $out/tests
        cp src/.libs/curl $out/src/curl 2>/dev/null || cp src/curl $out/src/curl
        if [ -f src/.libs/curlinfo ]; then cp src/.libs/curlinfo $out/src/curlinfo;
        elif [ -f src/curlinfo ]; then cp src/curlinfo $out/src/curlinfo; fi
        cp lib/.libs/libcurl.so* $out/lib/ 2>/dev/null || true
        chmod +x $out/tests/runtests.pl
      '';
      dontStrip = true;
    };
  in
    lib.warn "This derivation runs tests 1-200 in batch; use -L to see live output"
    (derivation {
      name = "rust-curl-test-discovery";
      inherit (stdenv) system;
      builder = "${stdenv.shell}";
      args = [
        "-c"
        ''
          export PATH="${lib.makeBinPath [perl coreutils diffutils gnused gnugrep stunnel rust-curl-dev]}"
          export TMPDIR=$(${coreutils}/bin/mktemp -d)
          export HOME="$TMPDIR"

          ${coreutils}/bin/cp -r "${curl-test-infra}/tests" "$TMPDIR/tests"
          ${coreutils}/bin/cp -r "${curl-test-infra}/src" "$TMPDIR/src"
          ${coreutils}/bin/chmod -R u+w "$TMPDIR/tests" "$TMPDIR/src"
          cd "$TMPDIR/tests"
          export LD_LIBRARY_PATH="${curl-test-infra}/lib"

          ${perl}/bin/perl ./runtests.pl \
            -c "${rust-curl-dev}/bin/curl" \
            -n \
            -a \
            1 to 200 \
            2>&1 | ${coreutils}/bin/tee "$TMPDIR/results.txt" || true

          ${coreutils}/bin/mkdir -p $out
          ${coreutils}/bin/cp "$TMPDIR/results.txt" $out/results.txt
        ''
      ];
      __darwinAllowLocalNetworking = true;
    });

  checks = let
    # Curated list of tests known to pass. Keep sorted ascending.
    # testsuite.nix fails the derivation unless the test ran and reported 100%
    # OK — so every number here is guaranteed to correspond to a real test.
    testNums = [
      1
      2
      3
      4
      5
      6
      7
      9
      10
      11
      12
      13
      14
      15
      16
      17
      18
      19
      20
      21
      22
      23
      24
      25
      26
      27
      28
      29
      30
      31
      32
      33
      34
      35
      36
      37
      38
      39
      40
      41
      42
      43
      44
      45
      46
      47
      49
      50
      51
      52
      53
      54
      55
      56
      57
      58
      59
      60
      61
      62
      63
      66
      71
      73
      74
      75
      77
      78
      80
      82
      83
      84
      85
      86
      87
      92
      93
      94
      95
      97
      98
      129
      151
      152
      156
      157
      158
      160
      163
      164
      166
      171
      172
      173
      174
      178
      179
      180
      181
      183
      184
      185
      186
      187
      188
      189
      192
      193
      194
      197
      198
      199
      218
      219
      220
      224
      232
      249
      256
      260
      262
      274
      276
      281
      282
      300
      301
      302
      303
      304
      306
      309
      310
      328
      331
      333
      334
      339
      341
      342
      343
      344
      345
      347
      349
      361
      364
      365
      367
      368
      371
      373
      374
      378
      383
      384
      391
      394
      395
      398
      410
      415
      419
      425
      426
      434
      443
      449
      452
      453
      454
      456
      460
      461
      462
      467
      468
      469
      470
      473
      485
      499
      502
      505
      511
      518
      520
      537
      540
      547
      548
      549
      550
      551
      552
      555
      558
      561
      567
      568
      569
      582
      583
      594
      600
      601
      603
      605
      632
      644
      646
      647
      648
      649
      660
      662
      663
      675
      677
      678
      686
      687
      688
      697
      708
      709
      710
      719
      722
      723
      724
      743
      752
      763
      767
      768
      769
      773
      787
      899
      979
      999
      1001
      1002
      1003
      1004
      1005
      1006
      1007
      1008
      1009
      1011
      1016
      1017
      1018
      1021
      1027
      1029
      1030
      1031
      1032
      1033
      1034
      1035
      1040
      1041
      1042
      1043
      1044
      1046
      1048
      1049
      1050
      1053
      1058
      1059
      1064
      1068
      1077
      1080
      1081
      1089
      1097
      1101
      1109
      1110
      1111
      1112
      1115
      1117
      1118
      1121
      1126
      1127
      1128
      1136
      1143
      1145
      1146
      1147
      1150
      1155
      1157
      1161
      1164
      1166
      1168
      1169
      1174
      1175
      1176
      1178
      1182
      1183
      1184
      1187
      1190
      1191
      1192
      1197
      1200
      1201
      1202
      1205
      1209
      1210
      1213
      1214
      1216
      1218
      1220
      1223
      1231
      1232
      1235
      1237
      1240
      1241
      1246
      1249
      1251
      1258
      1259
      1261
      1266
      1267
      1268
      1269
      1270
      1271
      1272
      1273
      1275
      1276
      1280
      1283
      1290
      1292
      1296
      1298
      1299
      1300
      1302
      1303
      1304
      1305
      1306
      1309
      1311
      1317
      1318
      1322
      1323
      1325
      1334
      1336
      1337
      1338
      1339
      1342
      1343
      1344
      1345
      1346
      1347
      1364
      1365
      1366
      1367
      1372
      1373
      1374
      1375
      1376
      1377
      1395
      1396
      1397
      1398
      1399
      1411
      1413
      1416
      1424
      1429
      1433
      1434
      1438
      1439
      1466
      1471
      1472
      1473
      1475
      1484
      1487
      1489
      1494
      1497
      1524
      1544
      1584
      1585
      1601
      1602
      1603
      1605
      1606
      1607
      1608
      1609
      1610
      1611
      1612
      1614
      1615
      1616
      1620
      1636
      1650
      1651
      1652
      1653
      1655
      1656
      1657
      1658
      1661
      1663
      1664
      1979
      1980
    ];
    allNums = testNums;
  in
    builtins.listToAttrs (map (num: {
        name = "rust-curl-test-${toString num}";
        value = pkgs:
          import ./testsuite.nix {
            inherit pkgs;
            testNum = num;
          };
      })
      allNums);
}
