//! The molecules `hacks3d::molecule` draws.
//!
//! Upstream generates `molecules.h` from the `.pdb` files in `hacks/images/`
//! at build time and compiles the text straight in. These are the same files,
//! byte for byte, included the same way; the saver parses them at startup as
//! upstream does.

/// Every bundled molecule, as its name and the text of its PDB file.
pub const MOLECULES: &[(&str, &str)] = &[
    ("adenine", include_str!("../molecules/adenine.pdb")),
    (
        "adrenochrome",
        include_str!("../molecules/adrenochrome.pdb"),
    ),
    ("bucky", include_str!("../molecules/bucky.pdb")),
    ("caffeine", include_str!("../molecules/caffeine.pdb")),
    ("capsaicin", include_str!("../molecules/capsaicin.pdb")),
    ("chlordecone", include_str!("../molecules/chlordecone.pdb")),
    ("cocaine", include_str!("../molecules/cocaine.pdb")),
    ("codeine", include_str!("../molecules/codeine.pdb")),
    ("cyclohexane", include_str!("../molecules/cyclohexane.pdb")),
    ("cytosine", include_str!("../molecules/cytosine.pdb")),
    ("dna", include_str!("../molecules/dna.pdb")),
    (
        "dodecahedrane",
        include_str!("../molecules/dodecahedrane.pdb"),
    ),
    ("dthc", include_str!("../molecules/dthc.pdb")),
    ("dynamite", include_str!("../molecules/dynamite.pdb")),
    ("glycol", include_str!("../molecules/glycol.pdb")),
    ("guanine", include_str!("../molecules/guanine.pdb")),
    ("heroin", include_str!("../molecules/heroin.pdb")),
    (
        "hexahelicene",
        include_str!("../molecules/hexahelicene.pdb"),
    ),
    ("ibuprofen", include_str!("../molecules/ibuprofen.pdb")),
    ("lsd", include_str!("../molecules/lsd.pdb")),
    ("menthol", include_str!("../molecules/menthol.pdb")),
    ("mescaline", include_str!("../molecules/mescaline.pdb")),
    (
        "methamphetamine",
        include_str!("../molecules/methamphetamine.pdb"),
    ),
    ("morphine", include_str!("../molecules/morphine.pdb")),
    ("nicotine", include_str!("../molecules/nicotine.pdb")),
    ("novocaine", include_str!("../molecules/novocaine.pdb")),
    ("olestra", include_str!("../molecules/olestra.pdb")),
    ("penicillin", include_str!("../molecules/penicillin.pdb")),
    ("salvinorin", include_str!("../molecules/salvinorin.pdb")),
    ("sarin", include_str!("../molecules/sarin.pdb")),
    ("strychnine", include_str!("../molecules/strychnine.pdb")),
    ("sucrose", include_str!("../molecules/sucrose.pdb")),
    ("thalidomide", include_str!("../molecules/thalidomide.pdb")),
    ("thymine", include_str!("../molecules/thymine.pdb")),
    ("viagra", include_str!("../molecules/viagra.pdb")),
    ("vitaminb6", include_str!("../molecules/vitaminb6.pdb")),
    ("vitaminc", include_str!("../molecules/vitaminc.pdb")),
    ("vx", include_str!("../molecules/vx.pdb")),
];
