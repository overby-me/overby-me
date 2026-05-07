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
    lib.warn "This derivation runs tests 1200-1500 in batch; use -L to see live output"
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
            700 to 900 \
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
      8
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
      207
      214
      218
      219
      220
      221
      222
      224
      230
      232
      233
      234
      249
      256
      260
      262
      264
      266
      269
      274
      276
      278
      279
      281
      282
      292
      293
      300
      301
      302
      303
      304
      305
      306
      309
      310
      317
      318
      319
      325
      326
      327
      328
      329
      330
      331
      333
      334
      339
      341
      342
      343
      344
      345
      346
      347
      349
      357
      360
      361
      364
      365
      366
      367
      368
      370
      371
      372
      373
      374
      376
      378
      379
      383
      384
      385
      386
      387
      389
      391
      392
      393
      394
      395
      398
      399
      410
      411
      415
      418
      419
      420
      421
      422
      425
      426
      434
      443
      444
      449
      452
      453
      454
      456
      460
      461
      462
      463
      467
      468
      469
      470
      473
      477
      481
      482
      484
      485
      497
      498
      499
      518
      537
      662
      663
      675
      678
      681
      686
      690
      691
      692
      693
      697
      708
      722
      723
      724
      743
      746
      747
      752
      759
      767
      768
      769
      770
      771
      772
      773
      778
      787
      794
      796
      797
      798
      898
      899
      977
      978
      979
      990
      991
      994
      995
      996
      998
      999
      1004
      1011
      1012
      1015
      1024
      1025
      1027
      1029
      1031
      1032
      1033
      1040
      1041
      1042
      1043
      1051
      1052
      1053
      1054
      1058
      1064
      1068
      1069
      1070
      1076
      1080
      1081
      1089
      1090
      1101
      1104
      1105
      1109
      1110
      1111
      1115
      1116
      1117
      1118
      1121
      1122
      1123
      1124
      1125
      1126
      1127
      1128
      1129
      1130
      1131
      1138
      1141
      1143
      1144
      1147
      1150
      1151
      1155
      1157
      1159
      1160
      1161
      1164
      1166
      1168
      1169
      1170
      1172
      1174
      1175
      1176
      1178
      1179
      1180
      1181
      1182
      1183
      1184
      1188
      1197
      1200
      1201
      1202
      1205
      1210
      1213
      1214
      1216
      1218
      1223
      1228
      1231
      1232
      1234
      1235
      1236
      1237
      1240
      1241
      1246
      1247
      1248
      1249
      1251
      1252
      1253
      1254
      1255
      1256
      1257
      1258
      1259
      1260
      1261
      1263
      1264
      1266
      1267
      1268
      1269
      1270
      1271
      1272
      1273
      1274
      1275
      1276
      1278
      1280
      1281
      1283
      1289
      1290
      1291
      1292
      1296
      1297
      1298
      1299
      1300
      1302
      1303
      1304
      1305
      1306
      1309
      1310
      1311
      1312
      1313
      1314
      1317
      1318
      1322
      1323
      1325
      1329
      1332
      1333
      1334
      1335
      1336
      1337
      1338
      1339
      1340
      1341
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
      1368
      1369
      1370
      1371
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
      1409
      1410
      1411
      1413
      1416
      1417
      1424
      1427
      1429
      1430
      1431
      1432
      1433
      1434
      1438
      1439
      1443
      1447
      1457
      1460
      1462
      1466
      1471
      1472
      1473
      1474
      1475
      1480
      1483
      1484
      1487
      1489
      1493
      1494
      1495
      1496
      1497
      1498
      1524
      1544
      1546
      1563
      1566
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
      1613
      1614
      1615
      1616
      1620
      1635
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
      1665
      1670
      1671
      1680
      1681
      1682
      1683
      1709
      1909
      1979
      1980
      2075
      2040
      2044
      2049
      2054
      2080
      2081
      2088
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
