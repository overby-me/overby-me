//! The three fish `atlantis` swims, from `hacks/glx/dolphin.c`, `shark.c` and
//! `whale.c`.
//!
//! ```text
//! atlantis --- Shows moving 3D sea animals
//!
//! Copyright (c) E. Lassauge, 1998.
//!
//! Permission to use, copy, modify, and distribute this software and its
//! documentation for any purpose and without fee is hereby granted,
//! provided that the above copyright notice appear in all copies and that
//! both that copyright notice and this permission notice appear in
//! supporting documentation.
//!
//! This file is provided AS IS with no warranties of any kind.  The author
//! shall have no liability with respect to the infringement of copyrights,
//! trade secrets or any patents by this file or any part thereof.  In no
//! event will the author be liable for any lost revenue or profits or
//! other special, indirect and consequential damages.
//!
//! The original code for this mode was written by Mark J. Kilgard
//! as a demo for openGL programming.
//!
//! (c) Copyright 1993, 1994, Silicon Graphics, Inc.
//! ALL RIGHTS RESERVED
//! Permission to use, copy, modify, and distribute this software for
//! any purpose and without fee is hereby granted, provided that the above
//! copyright notice appear in all copies and that both the copyright notice
//! and this permission notice appear in supporting documentation, and that
//! the name of Silicon Graphics, Inc. not be used in advertising
//! or publicity pertaining to distribution of the software without specific,
//! written prior permission.
//! ```
//!
//! Each fish is a table of face normals, a table of points, and a run of
//! functions that do nothing but name a normal and then name the three or
//! four points of a face. Upstream keeps the points in file-scope arrays and
//! writes the animated ones in place before drawing; here the table is a
//! local copy so that two fish can never be halfway through each other.
//!
//! Upstream opens a `glBegin` per face, four hundred of them a fish. A convex
//! polygon drawn as a fan of triangles is the same picture, so they all go
//! into one block instead and a fish costs one draw call rather than four
//! hundred.
//!
//! The water is the other thing worth knowing about. Upstream lays a noise
//! texture over everything with `GL_EYE_LINEAR` texture generation, which
//! takes a fragment's position in eye space and reads the texture at
//! `(x, z)`; its own OpenGL ES build has no texture generation and quietly
//! drops the effect. There is none here either, so the coordinates are worked
//! out per vertex in the port instead, which is what the fixed-function
//! pipeline would have done and gets the shimmer back.

use crate::runtime::Gl;
use crate::runtime::gl::{Mat4, Shape};

/// A fish's points, indexed by upstream's own numbering: `p[12]` is `P012`.
pub type Pts = [[f32; 3]];

/// `glTexGen` of `GL_EYE_LINEAR` with upstream's planes and texture matrix:
/// s is the eye-space x and t the eye-space z, both scaled down so that the
/// noise is spread across the whole tank rather than tiled per fish.
const TEXTURE_SCALE: f32 = 0.0005;

/// Somewhere for a fish to put its faces.
pub struct Fish<'a> {
    g: &'a mut Gl,
    wire: bool,
    textured: bool,
    /// The modelview as it stands, which is what the texture coordinates are
    /// worked out from.
    modelview: Mat4,
}

impl<'a> Fish<'a> {
    /// Opens a block. Call after the fish's own transforms are on the matrix
    /// stack, since the texture coordinates are read off it.
    pub fn new(g: &'a mut Gl, wire: bool, textured: bool) -> Fish<'a> {
        let modelview = g.glx.modelview_matrix();
        g.glx
            .begin(if wire { Shape::Lines } else { Shape::Triangles });
        Fish {
            g,
            wire,
            textured,
            modelview,
        }
    }

    pub fn normal(&mut self, n: [f32; 3]) {
        self.g.glx.normal3f(n[0], n[1], n[2]);
    }

    /// `glDisable(GL_DEPTH_TEST)` part way through a fish, which one of the
    /// dolphin's parts does. The block has to close and open again around it.
    pub fn depth_test(&mut self, on: bool) {
        let shape = if self.wire {
            Shape::Lines
        } else {
            Shape::Triangles
        };
        self.g.glx.end();
        self.g.glx.depth_test(on);
        self.g.glx.begin(shape);
    }

    fn vertex(&mut self, v: [f32; 3]) {
        if self.textured {
            let e = self.modelview.transform(v);
            // Textures here are the other way up from OpenGL's, so t is
            // flipped; on a tiling noise it makes no odds, but it is what the
            // rest of the ports do.
            self.g
                .glx
                .tex_coord2f(e[0] * TEXTURE_SCALE, 1.0 - e[2] * TEXTURE_SCALE);
        }
        self.g.glx.vertex3f(v[0], v[1], v[2]);
    }

    /// One face of the fish, as a fan of triangles or as its outline.
    pub fn poly(&mut self, verts: &[[f32; 3]]) {
        if self.wire {
            for i in 0..verts.len() {
                self.vertex(verts[i]);
                self.vertex(verts[(i + 1) % verts.len()]);
            }
            return;
        }
        for i in 1..verts.len() - 1 {
            self.vertex(verts[0]);
            self.vertex(verts[i]);
            self.vertex(verts[i + 1]);
        }
    }

    pub fn finish(self) {
        self.g.glx.end();
    }
}

/// `dolphin`'s face normals, upstream's `N001` and friends.
#[rustfmt::skip]
pub const DOLPHIN_N: [[f32; 3]; 135] = [
    [0.0, 0.0, 0.0],
    [-0.005937, -0.101998, -0.994767], // N001
    [0.93678, -0.200803, 0.286569], // N002
    [-0.233062, 0.972058, 0.028007], // N003
    [0.0, 1.0, 0.0], // N004
    [0.898117, 0.360171, 0.252315], // N005
    [-0.915437, 0.348456, 0.201378], // N006
    [0.602263, -0.777527, 0.18092], // N007
    [-0.906912, -0.412015, 0.088061], // N008
    [-0.015623, 0.999878, 0.0], // N009
    [0.0, -0.992278, 0.124035], // N010
    [0.0, -0.936329, -0.351123], // N011
    [0.884408, -0.429417, -0.182821], // N012
    [0.921121, 0.311084, -0.234016], // N013
    [0.382635, 0.877882, -0.287948], // N014
    [-0.380046, 0.888166, -0.258316], // N015
    [-0.891515, 0.392238, -0.226607], // N016
    [-0.901419, -0.382002, -0.203763], // N017
    [-0.367225, -0.911091, -0.187243], // N018
    [0.339539, -0.924846, -0.171388], // N019
    [0.914706, -0.378617, -0.14129], // N020
    [0.950662, 0.262713, -0.164994], // N021
    [0.546359, 0.80146, -0.243218], // N022
    [-0.315796, 0.917068, -0.243431], // N023
    [-0.825687, 0.532277, -0.186875], // N024
    [-0.974763, -0.155232, -0.160435], // N025
    [-0.560596, -0.816658, -0.137119], // N026
    [0.38021, -0.910817, -0.160786], // N027
    [0.923772, -0.358322, -0.135093], // N028
    [0.951202, 0.275053, -0.139859], // N029
    [0.686099, 0.702548, -0.188932], // N030
    [-0.521865, 0.826719, -0.21022], // N031
    [-0.92382, 0.346739, -0.162258], // N032
    [-0.902095, -0.409995, -0.134646], // N033
    [-0.509115, -0.848498, -0.144404], // N034
    [0.456469, -0.880293, -0.129305], // N035
    [0.873401, -0.475489, -0.105266], // N036
    [0.970825, 0.179861, -0.158584], // N037
    [0.675609, 0.714187, -0.183004], // N038
    [-0.523574, 0.830212, -0.19136], // N039
    [-0.958895, 0.230808, -0.165071], // N040
    [-0.918285, -0.376803, -0.121542], // N041
    [-0.622467, -0.774167, -0.114888], // N042
    [0.404497, -0.908807, -0.102231], // N043
    [0.930538, -0.365155, -0.027588], // N044
    [0.92192, 0.374157, -0.100345], // N045
    [0.507346, 0.860739, 0.041562], // N046
    [-0.394646, 0.918815, -0.00573], // N047
    [-0.925411, 0.373024, -0.066837], // N048
    [-0.945337, -0.322309, -0.049551], // N049
    [-0.660437, -0.750557, -0.022072], // N050
    [0.488835, -0.87195, -0.027261], // N051
    [0.902599, -0.421397, 0.087969], // N052
    [0.938636, 0.322606, 0.12202], // N053
    [0.484605, 0.871078, 0.079878], // N054
    [-0.353607, 0.931559, 0.084619], // N055
    [-0.867759, 0.478564, 0.134054], // N056
    [-0.951583, -0.29603, 0.082794], // N057
    [-0.672355, -0.730209, 0.121384], // N058
    [0.528336, -0.842452, 0.105525], // N059
    [0.786913, -0.56476, 0.248627], // N060
    [0.0, 1.0, 0.0], // N061
    [0.622098, 0.76523, 0.165584], // N062
    [-0.631711, 0.767816, 0.106773], // N063
    [-0.687886, 0.606351, 0.398938], // N064
    [-0.946327, -0.281623, 0.158598], // N065
    [-0.509549, -0.860437, 0.002776], // N066
    [0.462594, -0.876692, 0.131977], // N067
    [0.0, -0.992278, 0.124035], // N068
    [0.0, -0.970143, -0.242536], // N069
    [0.015502, 0.992159, -0.12402], // N070
    [0.0, 1.0, 0.0], // N071
    [0.0, 1.0, 0.0], // N072
    [0.0, 1.0, 0.0], // N073
    [0.0, -1.0, 0.0], // N074
    [-0.242536, 0.0, -0.970143], // N075
    [-0.010336, -0.992225, -0.124028], // N076
    [-0.88077, 0.461448, 0.106351], // N077
    [-0.88077, 0.461448, 0.106351], // N078
    [-0.88077, 0.461448, 0.106351], // N079
    [-0.88077, 0.461448, 0.106351], // N080
    [-0.571197, 0.816173, 0.087152], // N081
    [-0.88077, 0.461448, 0.106351], // N082
    [-0.571197, 0.816173, 0.087152], // N083
    [-0.571197, 0.816173, 0.087152], // N084
    [-0.88077, 0.461448, 0.106351], // N085
    [-0.571197, 0.816173, 0.087152], // N086
    [-0.88077, 0.461448, 0.106351], // N087
    [-0.88077, 0.461448, 0.106351], // N088
    [-0.88077, 0.461448, 0.106351], // N089
    [-0.88077, 0.461448, 0.106351], // N090
    [0.0, 1.0, 0.0], // N091
    [0.0, 1.0, 0.0], // N092
    [0.0, 1.0, 0.0], // N093
    [1.0, 0.0, 0.0], // N094
    [-1.0, 0.0, 0.0], // N095
    [0.0, 1.0, 0.0], // N096
    [-0.697296, 0.702881, 0.140491], // N097
    [0.918864, 0.340821, 0.198819], // N098
    [-0.932737, 0.201195, 0.299202], // N099
    [0.029517, 0.981679, 0.188244], // N100
    [0.0, 1.0, 0.0], // N101
    [0.813521, -0.204936, 0.544229], // N102
    [0.0, 1.0, 0.0], // N103
    [0.0, 1.0, 0.0], // N104
    [0.0, 1.0, 0.0], // N105
    [0.0, 1.0, 0.0], // N106
    [0.0, 1.0, 0.0], // N107
    [0.0, 1.0, 0.0], // N108
    [0.0, 1.0, 0.0], // N109
    [-0.78148, -0.384779, 0.491155], // N110
    [-0.722243, 0.384927, 0.574627], // N111
    [-0.752278, 0.502679, 0.425901], // N112
    [0.547257, 0.36791, 0.751766], // N113
    [0.725949, -0.232568, 0.647233], // N114
    [-0.747182, -0.660786, 0.07128], // N115
    [0.931519, 0.200748, 0.30327], // N116
    [-0.828928, 0.313757, 0.463071], // N117
    [0.902554, -0.370967, 0.218587], // N118
    [-0.879257, -0.441851, 0.177973], // N119
    [0.642327, 0.611901, 0.461512], // N120
    [0.964817, -0.202322, 0.16791], // N121
    [0.0, 1.0, 0.0], // N122
    [-0.980734, 0.041447, 0.1909], // N123
    [-0.980734, 0.041447, 0.1909], // N124
    [-0.980734, 0.041447, 0.1909], // N125
    [0.0, 1.0, 0.0], // N126
    [0.0, 1.0, 0.0], // N127
    [0.0, 1.0, 0.0], // N128
    [0.96325, 0.004839, 0.268565], // N129
    [0.96325, 0.004839, 0.268565], // N130
    [0.96325, 0.004839, 0.268565], // N131
    [0.0, 1.0, 0.0], // N132
    [0.0, 1.0, 0.0], // N133
    [0.0, 1.0, 0.0], // N134
];

/// `dolphin`'s points at rest, which are upstream's `P001` and
/// its immutable copy `iP001` both: the two are declared with the same
/// values and only the working copy is ever written to.
#[rustfmt::skip]
pub const DOLPHIN_P: [[f32; 3]; 135] = [
    [0.0, 0.0, 0.0],
    [5.68, -300.95, 1324.7], // P001
    [338.69, -219.63, 9677.03], // P002
    [12.18, 474.59, 9138.14], // P003
    [-7.49, -388.91, 10896.74], // P004
    [487.51, 198.05, 9350.78], // P005
    [-457.61, 68.74, 9427.85], // P006
    [156.52, -266.72, 10311.68], // P007
    [-185.56, -266.51, 10310.47], // P008
    [124.39, -261.46, 1942.34], // P009
    [-130.05, -261.46, 1946.03], // P010
    [141.07, -320.11, 1239.38], // P011
    [156.48, -360.12, 2073.41], // P012
    [162.0, -175.88, 2064.44], // P013
    [88.16, -87.72, 2064.02], // P014
    [-65.21, -96.13, 2064.02], // P015
    [-156.48, -180.96, 2064.44], // P016
    [-162.0, -368.93, 2082.39], // P017
    [-88.16, -439.22, 2082.39], // P018
    [65.21, -440.32, 2083.39], // P019
    [246.87, -356.02, 2576.95], // P020
    [253.17, -111.15, 2567.15], // P021
    [132.34, 51.41, 2559.84], // P022
    [-97.88, 40.44, 2567.15], // P023
    [-222.97, -117.49, 2567.15], // P024
    [-252.22, -371.53, 2569.92], // P025
    [-108.44, -518.19, 2586.75], // P026
    [97.88, -524.79, 2586.75], // P027
    [370.03, -421.19, 3419.7], // P028
    [351.15, -16.98, 3423.17], // P029
    [200.66, 248.46, 3430.37], // P030
    [-148.42, 235.02, 3417.91], // P031
    [-360.21, -30.27, 3416.84], // P032
    [-357.9, -414.89, 3407.04], // P033
    [-148.88, -631.35, 3409.9], // P034
    [156.38, -632.59, 3419.7], // P035
    [462.61, -469.21, 4431.51], // P036
    [466.6, 102.25, 4434.98], // P037
    [243.05, 474.34, 4562.02], // P038
    [-191.23, 474.4, 4554.42], // P039
    [-476.12, 111.05, 4451.11], // P040
    [-473.36, -470.74, 4444.78], // P041
    [-266.95, -748.41, 4447.78], // P042
    [211.14, -749.91, 4429.73], // P043
    [680.57, -370.27, 5943.46], // P044
    [834.01, 363.09, 6360.63], // P045
    [371.29, 804.51, 6486.26], // P046
    [-291.43, 797.22, 6494.28], // P047
    [-784.13, 370.75, 6378.01], // P048
    [-743.29, -325.82, 5943.46], // P049
    [-383.24, -804.77, 5943.46], // P050
    [283.47, -846.09, 5943.46], // P051
    [599.09, -300.15, 7894.03], // P052
    [735.48, 306.26, 7911.92], // P053
    [246.22, 558.53, 8460.5], // P054
    [-230.41, 559.84, 8473.23], // P055
    [-698.66, 320.83, 7902.59], // P056
    [-643.29, -299.16, 7902.59], // P057
    [-341.47, -719.3, 7902.59], // P058
    [252.57, -756.12, 7902.59], // P059
    [458.39, -265.31, 9355.44], // P060
    [433.38, -161.9, 9503.03], // P061
    [224.04, 338.75, 9450.3], // P062
    [-165.71, 341.04, 9462.35], // P063
    [-298.11, 110.13, 10180.37], // P064
    [-473.99, -219.71, 9355.44], // P065
    [-211.97, -479.87, 9355.44], // P066
    [192.86, -491.45, 9348.73], // P067
    [-136.29, -319.84, 1228.73], // P068
    [1111.17, -314.14, 1314.19], // P069
    [-1167.34, -321.61, 1319.45], // P070
    [1404.86, -306.66, 1235.45], // P071
    [-1409.73, -314.14, 1247.66], // P072
    [1254.01, -296.87, 1544.58], // P073
    [-1262.09, -291.7, 1504.26], // P074
    [965.71, -269.26, 1742.65], // P075
    [-900.97, -276.74, 1726.07], // P076
    [1058.0, -448.81, 8194.66], // P077
    [-1016.51, -456.43, 8190.62], // P078
    [-1515.96, -676.45, 7754.93], // P079
    [1856.75, -830.34, 7296.56], // P080
    [1472.16, -497.38, 7399.68], // P081
    [-1775.26, -829.51, 7298.46], // P082
    [911.09, -252.51, 7510.99], // P083
    [-1451.94, -495.62, 7384.3], // P084
    [1598.75, -669.26, 7769.9], // P085
    [-836.53, -250.08, 7463.25], // P086
    [722.87, -158.18, 8006.41], // P087
    [-688.86, -162.28, 7993.89], // P088
    [-626.92, -185.3, 8364.98], // P089
    [647.72, -189.46, 8354.99], // P090
    [0.0, 835.01, 5555.62], // P091
    [0.0, 1350.18, 5220.86], // P092
    [0.0, 1422.94, 5285.27], // P093
    [0.0, 1296.75, 5650.19], // P094
    [0.0, 795.63, 6493.88], // P095
    [-447.38, -165.99, 9499.6], // P096
    [-194.91, -357.14, 10313.32], // P097
    [135.35, -357.66, 10307.94], // P098
    [-380.53, -221.14, 9677.98], // P099
    [0.0, 412.99, 9629.33], // P100
    [5.7, 567.0, 7862.98], // P101
    [59.51, -412.55, 10677.58], // P102
    [6.5, 484.74, 9009.94], // P103
    [-9.86, 567.62, 7858.65], // P104
    [-41.86, 476.51, 9078.17], // P105
    [22.75, 568.13, 7782.83], // P106
    [58.93, 568.42, 7775.94], // P107
    [49.2, 476.83, 9078.24], // P108
    [99.21, 566.0, 7858.65], // P109
    [-187.62, -410.04, 10674.12], // P110
    [-184.25, -318.7, 10723.88], // P111
    [-179.61, -142.81, 10670.26], // P112
    [57.43, -147.94, 10675.26], // P113
    [54.06, -218.9, 10712.44], // P114
    [-186.35, -212.09, 10713.76], // P115
    [205.9, -84.61, 10275.97], // P116
    [-230.96, -83.26, 10280.09], // P117
    [216.78, -509.17, 10098.94], // P118
    [-313.21, -510.79, 10102.62], // P119
    [217.95, 96.34, 10161.62], // P120
    [71.99, -319.74, 10717.7], // P121
    [0.0, 602.74, 5375.84], // P122
    [-448.94, -203.14, 9499.6], // P123
    [-442.64, -185.2, 9528.07], // P124
    [-441.07, -148.05, 9528.07], // P125
    [-443.43, -128.84, 9499.6], // P126
    [-456.87, -146.78, 9466.67], // P127
    [-453.68, -183.93, 9466.67], // P128
    [428.43, -124.08, 9503.03], // P129
    [419.73, -142.14, 9534.56], // P130
    [419.92, -179.96, 9534.56], // P131
    [431.2, -199.73, 9505.26], // P132
    [442.28, -181.67, 9475.96], // P133
    [442.08, -143.84, 9475.96], // P134
];

pub fn dolphin001(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[71]);
    f.poly(&[p[1], p[68], p[10]]);
    f.poly(&[p[68], p[76], p[10]]);
    f.poly(&[p[68], p[70], p[76]]);
    f.poly(&[p[76], p[70], p[74]]);
    f.poly(&[p[70], p[72], p[74]]);
    f.normal(DOLPHIN_N[119]);
    f.poly(&[p[72], p[70], p[74]]);
    f.poly(&[p[74], p[70], p[76]]);
    f.poly(&[p[70], p[68], p[76]]);
    f.poly(&[p[76], p[68], p[10]]);
    f.poly(&[p[68], p[1], p[10]]);
}

pub fn dolphin002(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[71]);
    f.poly(&[p[11], p[1], p[9]]);
    f.poly(&[p[75], p[11], p[9]]);
    f.poly(&[p[69], p[11], p[75]]);
    f.poly(&[p[69], p[75], p[73]]);
    f.poly(&[p[71], p[69], p[73]]);
    f.normal(DOLPHIN_N[119]);
    f.poly(&[p[1], p[11], p[9]]);
    f.poly(&[p[9], p[11], p[75]]);
    f.poly(&[p[11], p[69], p[75]]);
    f.poly(&[p[69], p[73], p[75]]);
    f.poly(&[p[69], p[71], p[73]]);
}

pub fn dolphin003(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[18]);
    f.normal(DOLPHIN_N[1]);
    f.normal(DOLPHIN_N[19]);
    f.poly(&[p[18], p[1], p[19]]);
    f.normal(DOLPHIN_N[19]);
    f.normal(DOLPHIN_N[1]);
    f.normal(DOLPHIN_N[12]);
    f.poly(&[p[19], p[1], p[12]]);
    f.normal(DOLPHIN_N[17]);
    f.normal(DOLPHIN_N[1]);
    f.normal(DOLPHIN_N[18]);
    f.poly(&[p[17], p[1], p[18]]);
    f.normal(DOLPHIN_N[1]);
    f.normal(DOLPHIN_N[17]);
    f.normal(DOLPHIN_N[16]);
    f.poly(&[p[1], p[17], p[16]]);
    f.normal(DOLPHIN_N[1]);
    f.normal(DOLPHIN_N[13]);
    f.normal(DOLPHIN_N[12]);
    f.poly(&[p[1], p[13], p[12]]);
    f.normal(DOLPHIN_N[1]);
    f.normal(DOLPHIN_N[16]);
    f.normal(DOLPHIN_N[15]);
    f.poly(&[p[1], p[16], p[15]]);
    f.normal(DOLPHIN_N[1]);
    f.normal(DOLPHIN_N[14]);
    f.normal(DOLPHIN_N[13]);
    f.poly(&[p[1], p[14], p[13]]);
    f.normal(DOLPHIN_N[1]);
    f.normal(DOLPHIN_N[15]);
    f.normal(DOLPHIN_N[14]);
    f.poly(&[p[1], p[15], p[14]]);
}

pub fn dolphin004(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[14]);
    f.normal(DOLPHIN_N[15]);
    f.normal(DOLPHIN_N[23]);
    f.normal(DOLPHIN_N[22]);
    f.poly(&[p[14], p[15], p[23], p[22]]);
    f.normal(DOLPHIN_N[15]);
    f.normal(DOLPHIN_N[16]);
    f.normal(DOLPHIN_N[24]);
    f.normal(DOLPHIN_N[23]);
    f.poly(&[p[15], p[16], p[24], p[23]]);
    f.normal(DOLPHIN_N[16]);
    f.normal(DOLPHIN_N[17]);
    f.normal(DOLPHIN_N[25]);
    f.normal(DOLPHIN_N[24]);
    f.poly(&[p[16], p[17], p[25], p[24]]);
    f.normal(DOLPHIN_N[17]);
    f.normal(DOLPHIN_N[18]);
    f.normal(DOLPHIN_N[26]);
    f.normal(DOLPHIN_N[25]);
    f.poly(&[p[17], p[18], p[26], p[25]]);
    f.normal(DOLPHIN_N[13]);
    f.normal(DOLPHIN_N[14]);
    f.normal(DOLPHIN_N[22]);
    f.normal(DOLPHIN_N[21]);
    f.poly(&[p[13], p[14], p[22], p[21]]);
    f.normal(DOLPHIN_N[12]);
    f.normal(DOLPHIN_N[13]);
    f.normal(DOLPHIN_N[21]);
    f.normal(DOLPHIN_N[20]);
    f.poly(&[p[12], p[13], p[21], p[20]]);
    f.normal(DOLPHIN_N[18]);
    f.normal(DOLPHIN_N[19]);
    f.normal(DOLPHIN_N[27]);
    f.normal(DOLPHIN_N[26]);
    f.poly(&[p[18], p[19], p[27], p[26]]);
    f.normal(DOLPHIN_N[19]);
    f.normal(DOLPHIN_N[12]);
    f.normal(DOLPHIN_N[20]);
    f.normal(DOLPHIN_N[27]);
    f.poly(&[p[19], p[12], p[20], p[27]]);
}

pub fn dolphin005(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[22]);
    f.normal(DOLPHIN_N[23]);
    f.normal(DOLPHIN_N[31]);
    f.normal(DOLPHIN_N[30]);
    f.poly(&[p[22], p[23], p[31], p[30]]);
    f.normal(DOLPHIN_N[21]);
    f.normal(DOLPHIN_N[22]);
    f.normal(DOLPHIN_N[30]);
    f.poly(&[p[21], p[22], p[30]]);
    f.normal(DOLPHIN_N[21]);
    f.normal(DOLPHIN_N[30]);
    f.normal(DOLPHIN_N[29]);
    f.poly(&[p[21], p[30], p[29]]);
    f.normal(DOLPHIN_N[23]);
    f.normal(DOLPHIN_N[24]);
    f.normal(DOLPHIN_N[31]);
    f.poly(&[p[23], p[24], p[31]]);
    f.normal(DOLPHIN_N[24]);
    f.normal(DOLPHIN_N[32]);
    f.normal(DOLPHIN_N[31]);
    f.poly(&[p[24], p[32], p[31]]);
    f.normal(DOLPHIN_N[24]);
    f.normal(DOLPHIN_N[25]);
    f.normal(DOLPHIN_N[32]);
    f.poly(&[p[24], p[25], p[32]]);
    f.normal(DOLPHIN_N[25]);
    f.normal(DOLPHIN_N[33]);
    f.normal(DOLPHIN_N[32]);
    f.poly(&[p[25], p[33], p[32]]);
    f.normal(DOLPHIN_N[20]);
    f.normal(DOLPHIN_N[21]);
    f.normal(DOLPHIN_N[29]);
    f.poly(&[p[20], p[21], p[29]]);
    f.normal(DOLPHIN_N[20]);
    f.normal(DOLPHIN_N[29]);
    f.normal(DOLPHIN_N[28]);
    f.poly(&[p[20], p[29], p[28]]);
    f.normal(DOLPHIN_N[27]);
    f.normal(DOLPHIN_N[20]);
    f.normal(DOLPHIN_N[28]);
    f.poly(&[p[27], p[20], p[28]]);
    f.normal(DOLPHIN_N[27]);
    f.normal(DOLPHIN_N[28]);
    f.normal(DOLPHIN_N[35]);
    f.poly(&[p[27], p[28], p[35]]);
    f.normal(DOLPHIN_N[25]);
    f.normal(DOLPHIN_N[26]);
    f.normal(DOLPHIN_N[33]);
    f.poly(&[p[25], p[26], p[33]]);
    f.normal(DOLPHIN_N[33]);
    f.normal(DOLPHIN_N[26]);
    f.normal(DOLPHIN_N[34]);
    f.poly(&[p[33], p[26], p[34]]);
    f.normal(DOLPHIN_N[26]);
    f.normal(DOLPHIN_N[27]);
    f.normal(DOLPHIN_N[35]);
    f.normal(DOLPHIN_N[34]);
    f.poly(&[p[26], p[27], p[35], p[34]]);
}

pub fn dolphin006(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[92]);
    f.normal(DOLPHIN_N[93]);
    f.normal(DOLPHIN_N[94]);
    f.poly(&[p[92], p[93], p[94]]);
    f.normal(DOLPHIN_N[93]);
    f.normal(DOLPHIN_N[92]);
    f.normal(DOLPHIN_N[94]);
    f.poly(&[p[93], p[92], p[94]]);
    f.normal(DOLPHIN_N[92]);
    f.normal(DOLPHIN_N[91]);
    f.normal(DOLPHIN_N[95]);
    f.normal(DOLPHIN_N[94]);
    f.poly(&[p[92], p[91], p[95], p[94]]);
    f.normal(DOLPHIN_N[91]);
    f.normal(DOLPHIN_N[92]);
    f.normal(DOLPHIN_N[94]);
    f.normal(DOLPHIN_N[95]);
    f.poly(&[p[91], p[92], p[94], p[95]]);
    f.normal(DOLPHIN_N[122]);
    f.normal(DOLPHIN_N[95]);
    f.normal(DOLPHIN_N[91]);
    f.poly(&[p[122], p[95], p[91]]);
    f.normal(DOLPHIN_N[122]);
    f.normal(DOLPHIN_N[91]);
    f.normal(DOLPHIN_N[95]);
    f.poly(&[p[122], p[91], p[95]]);
}

pub fn dolphin007(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[30]);
    f.normal(DOLPHIN_N[31]);
    f.normal(DOLPHIN_N[39]);
    f.normal(DOLPHIN_N[38]);
    f.poly(&[p[30], p[31], p[39], p[38]]);
    f.normal(DOLPHIN_N[29]);
    f.normal(DOLPHIN_N[30]);
    f.normal(DOLPHIN_N[38]);
    f.poly(&[p[29], p[30], p[38]]);
    f.normal(DOLPHIN_N[29]);
    f.normal(DOLPHIN_N[38]);
    f.normal(DOLPHIN_N[37]);
    f.poly(&[p[29], p[38], p[37]]);
    f.normal(DOLPHIN_N[28]);
    f.normal(DOLPHIN_N[29]);
    f.normal(DOLPHIN_N[37]);
    f.poly(&[p[28], p[29], p[37]]);
    f.normal(DOLPHIN_N[28]);
    f.normal(DOLPHIN_N[37]);
    f.normal(DOLPHIN_N[36]);
    f.poly(&[p[28], p[37], p[36]]);
    f.normal(DOLPHIN_N[35]);
    f.normal(DOLPHIN_N[28]);
    f.normal(DOLPHIN_N[36]);
    f.poly(&[p[35], p[28], p[36]]);
    f.normal(DOLPHIN_N[35]);
    f.normal(DOLPHIN_N[36]);
    f.normal(DOLPHIN_N[43]);
    f.poly(&[p[35], p[36], p[43]]);
    f.normal(DOLPHIN_N[34]);
    f.normal(DOLPHIN_N[35]);
    f.normal(DOLPHIN_N[43]);
    f.normal(DOLPHIN_N[42]);
    f.poly(&[p[34], p[35], p[43], p[42]]);
    f.normal(DOLPHIN_N[33]);
    f.normal(DOLPHIN_N[34]);
    f.normal(DOLPHIN_N[42]);
    f.poly(&[p[33], p[34], p[42]]);
    f.normal(DOLPHIN_N[33]);
    f.normal(DOLPHIN_N[42]);
    f.normal(DOLPHIN_N[41]);
    f.poly(&[p[33], p[42], p[41]]);
    f.normal(DOLPHIN_N[31]);
    f.normal(DOLPHIN_N[32]);
    f.normal(DOLPHIN_N[39]);
    f.poly(&[p[31], p[32], p[39]]);
    f.normal(DOLPHIN_N[39]);
    f.normal(DOLPHIN_N[32]);
    f.normal(DOLPHIN_N[40]);
    f.poly(&[p[39], p[32], p[40]]);
    f.normal(DOLPHIN_N[32]);
    f.normal(DOLPHIN_N[33]);
    f.normal(DOLPHIN_N[40]);
    f.poly(&[p[32], p[33], p[40]]);
    f.normal(DOLPHIN_N[40]);
    f.normal(DOLPHIN_N[33]);
    f.normal(DOLPHIN_N[41]);
    f.poly(&[p[40], p[33], p[41]]);
}

pub fn dolphin008(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[42]);
    f.normal(DOLPHIN_N[43]);
    f.normal(DOLPHIN_N[51]);
    f.normal(DOLPHIN_N[50]);
    f.poly(&[p[42], p[43], p[51], p[50]]);
    f.normal(DOLPHIN_N[43]);
    f.normal(DOLPHIN_N[36]);
    f.normal(DOLPHIN_N[51]);
    f.poly(&[p[43], p[36], p[51]]);
    f.normal(DOLPHIN_N[51]);
    f.normal(DOLPHIN_N[36]);
    f.normal(DOLPHIN_N[44]);
    f.poly(&[p[51], p[36], p[44]]);
    f.normal(DOLPHIN_N[41]);
    f.normal(DOLPHIN_N[42]);
    f.normal(DOLPHIN_N[50]);
    f.poly(&[p[41], p[42], p[50]]);
    f.normal(DOLPHIN_N[41]);
    f.normal(DOLPHIN_N[50]);
    f.normal(DOLPHIN_N[49]);
    f.poly(&[p[41], p[50], p[49]]);
    f.normal(DOLPHIN_N[36]);
    f.normal(DOLPHIN_N[37]);
    f.normal(DOLPHIN_N[44]);
    f.poly(&[p[36], p[37], p[44]]);
    f.normal(DOLPHIN_N[44]);
    f.normal(DOLPHIN_N[37]);
    f.normal(DOLPHIN_N[45]);
    f.poly(&[p[44], p[37], p[45]]);
    f.normal(DOLPHIN_N[40]);
    f.normal(DOLPHIN_N[41]);
    f.normal(DOLPHIN_N[49]);
    f.poly(&[p[40], p[41], p[49]]);
    f.normal(DOLPHIN_N[40]);
    f.normal(DOLPHIN_N[49]);
    f.normal(DOLPHIN_N[48]);
    f.poly(&[p[40], p[49], p[48]]);
    f.normal(DOLPHIN_N[39]);
    f.normal(DOLPHIN_N[40]);
    f.normal(DOLPHIN_N[48]);
    f.poly(&[p[39], p[40], p[48]]);
    f.normal(DOLPHIN_N[39]);
    f.normal(DOLPHIN_N[48]);
    f.normal(DOLPHIN_N[47]);
    f.poly(&[p[39], p[48], p[47]]);
    f.normal(DOLPHIN_N[37]);
    f.normal(DOLPHIN_N[38]);
    f.normal(DOLPHIN_N[45]);
    f.poly(&[p[37], p[38], p[45]]);
    f.normal(DOLPHIN_N[38]);
    f.normal(DOLPHIN_N[46]);
    f.normal(DOLPHIN_N[45]);
    f.poly(&[p[38], p[46], p[45]]);
    f.normal(DOLPHIN_N[38]);
    f.normal(DOLPHIN_N[39]);
    f.normal(DOLPHIN_N[47]);
    f.normal(DOLPHIN_N[46]);
    f.poly(&[p[38], p[39], p[47], p[46]]);
}

pub fn dolphin009(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[50]);
    f.normal(DOLPHIN_N[51]);
    f.normal(DOLPHIN_N[59]);
    f.normal(DOLPHIN_N[58]);
    f.poly(&[p[50], p[51], p[59], p[58]]);
    f.normal(DOLPHIN_N[51]);
    f.normal(DOLPHIN_N[44]);
    f.normal(DOLPHIN_N[59]);
    f.poly(&[p[51], p[44], p[59]]);
    f.normal(DOLPHIN_N[59]);
    f.normal(DOLPHIN_N[44]);
    f.normal(DOLPHIN_N[52]);
    f.poly(&[p[59], p[44], p[52]]);
    f.normal(DOLPHIN_N[44]);
    f.normal(DOLPHIN_N[45]);
    f.normal(DOLPHIN_N[53]);
    f.poly(&[p[44], p[45], p[53]]);
    f.normal(DOLPHIN_N[44]);
    f.normal(DOLPHIN_N[53]);
    f.normal(DOLPHIN_N[52]);
    f.poly(&[p[44], p[53], p[52]]);
    f.normal(DOLPHIN_N[49]);
    f.normal(DOLPHIN_N[50]);
    f.normal(DOLPHIN_N[58]);
    f.poly(&[p[49], p[50], p[58]]);
    f.normal(DOLPHIN_N[49]);
    f.normal(DOLPHIN_N[58]);
    f.normal(DOLPHIN_N[57]);
    f.poly(&[p[49], p[58], p[57]]);
    f.normal(DOLPHIN_N[48]);
    f.normal(DOLPHIN_N[49]);
    f.normal(DOLPHIN_N[57]);
    f.poly(&[p[48], p[49], p[57]]);
    f.normal(DOLPHIN_N[48]);
    f.normal(DOLPHIN_N[57]);
    f.normal(DOLPHIN_N[56]);
    f.poly(&[p[48], p[57], p[56]]);
    f.normal(DOLPHIN_N[47]);
    f.normal(DOLPHIN_N[48]);
    f.normal(DOLPHIN_N[56]);
    f.poly(&[p[47], p[48], p[56]]);
    f.normal(DOLPHIN_N[47]);
    f.normal(DOLPHIN_N[56]);
    f.normal(DOLPHIN_N[55]);
    f.poly(&[p[47], p[56], p[55]]);
    f.normal(DOLPHIN_N[45]);
    f.normal(DOLPHIN_N[46]);
    f.normal(DOLPHIN_N[53]);
    f.poly(&[p[45], p[46], p[53]]);
    f.normal(DOLPHIN_N[46]);
    f.normal(DOLPHIN_N[54]);
    f.normal(DOLPHIN_N[53]);
    f.poly(&[p[46], p[54], p[53]]);
    f.normal(DOLPHIN_N[46]);
    f.normal(DOLPHIN_N[47]);
    f.normal(DOLPHIN_N[55]);
    f.normal(DOLPHIN_N[54]);
    f.poly(&[p[46], p[47], p[55], p[54]]);
}

pub fn dolphin010(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[80]);
    f.normal(DOLPHIN_N[81]);
    f.normal(DOLPHIN_N[85]);
    f.poly(&[p[80], p[81], p[85]]);
    f.normal(DOLPHIN_N[81]);
    f.normal(DOLPHIN_N[83]);
    f.normal(DOLPHIN_N[85]);
    f.poly(&[p[81], p[83], p[85]]);
    f.normal(DOLPHIN_N[85]);
    f.normal(DOLPHIN_N[83]);
    f.normal(DOLPHIN_N[77]);
    f.poly(&[p[85], p[83], p[77]]);
    f.normal(DOLPHIN_N[83]);
    f.normal(DOLPHIN_N[87]);
    f.normal(DOLPHIN_N[77]);
    f.poly(&[p[83], p[87], p[77]]);
    f.normal(DOLPHIN_N[77]);
    f.normal(DOLPHIN_N[87]);
    f.normal(DOLPHIN_N[90]);
    f.poly(&[p[77], p[87], p[90]]);
    f.normal(DOLPHIN_N[81]);
    f.normal(DOLPHIN_N[80]);
    f.normal(DOLPHIN_N[85]);
    f.poly(&[p[81], p[80], p[85]]);
    f.normal(DOLPHIN_N[83]);
    f.normal(DOLPHIN_N[81]);
    f.normal(DOLPHIN_N[85]);
    f.poly(&[p[83], p[81], p[85]]);
    f.normal(DOLPHIN_N[83]);
    f.normal(DOLPHIN_N[85]);
    f.normal(DOLPHIN_N[77]);
    f.poly(&[p[83], p[85], p[77]]);
    f.normal(DOLPHIN_N[87]);
    f.normal(DOLPHIN_N[83]);
    f.normal(DOLPHIN_N[77]);
    f.poly(&[p[87], p[83], p[77]]);
    f.normal(DOLPHIN_N[87]);
    f.normal(DOLPHIN_N[77]);
    f.normal(DOLPHIN_N[90]);
    f.poly(&[p[87], p[77], p[90]]);
}

pub fn dolphin011(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[82]);
    f.normal(DOLPHIN_N[84]);
    f.normal(DOLPHIN_N[79]);
    f.poly(&[p[82], p[84], p[79]]);
    f.normal(DOLPHIN_N[84]);
    f.normal(DOLPHIN_N[86]);
    f.normal(DOLPHIN_N[79]);
    f.poly(&[p[84], p[86], p[79]]);
    f.normal(DOLPHIN_N[79]);
    f.normal(DOLPHIN_N[86]);
    f.normal(DOLPHIN_N[78]);
    f.poly(&[p[79], p[86], p[78]]);
    f.normal(DOLPHIN_N[86]);
    f.normal(DOLPHIN_N[88]);
    f.normal(DOLPHIN_N[78]);
    f.poly(&[p[86], p[88], p[78]]);
    f.normal(DOLPHIN_N[78]);
    f.normal(DOLPHIN_N[88]);
    f.normal(DOLPHIN_N[89]);
    f.poly(&[p[78], p[88], p[89]]);
    f.normal(DOLPHIN_N[88]);
    f.normal(DOLPHIN_N[86]);
    f.normal(DOLPHIN_N[89]);
    f.poly(&[p[88], p[86], p[89]]);
    f.normal(DOLPHIN_N[89]);
    f.normal(DOLPHIN_N[86]);
    f.normal(DOLPHIN_N[78]);
    f.poly(&[p[89], p[86], p[78]]);
    f.normal(DOLPHIN_N[86]);
    f.normal(DOLPHIN_N[84]);
    f.normal(DOLPHIN_N[78]);
    f.poly(&[p[86], p[84], p[78]]);
    f.normal(DOLPHIN_N[78]);
    f.normal(DOLPHIN_N[84]);
    f.normal(DOLPHIN_N[79]);
    f.poly(&[p[78], p[84], p[79]]);
    f.normal(DOLPHIN_N[84]);
    f.normal(DOLPHIN_N[82]);
    f.normal(DOLPHIN_N[79]);
    f.poly(&[p[84], p[82], p[79]]);
}

pub fn dolphin012(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[58]);
    f.normal(DOLPHIN_N[59]);
    f.normal(DOLPHIN_N[67]);
    f.normal(DOLPHIN_N[66]);
    f.poly(&[p[58], p[59], p[67], p[66]]);
    f.normal(DOLPHIN_N[59]);
    f.normal(DOLPHIN_N[52]);
    f.normal(DOLPHIN_N[60]);
    f.poly(&[p[59], p[52], p[60]]);
    f.normal(DOLPHIN_N[59]);
    f.normal(DOLPHIN_N[60]);
    f.normal(DOLPHIN_N[67]);
    f.poly(&[p[59], p[60], p[67]]);
    f.normal(DOLPHIN_N[58]);
    f.normal(DOLPHIN_N[66]);
    f.normal(DOLPHIN_N[65]);
    f.poly(&[p[58], p[66], p[65]]);
    f.normal(DOLPHIN_N[58]);
    f.normal(DOLPHIN_N[65]);
    f.normal(DOLPHIN_N[57]);
    f.poly(&[p[58], p[65], p[57]]);
    f.normal(DOLPHIN_N[56]);
    f.normal(DOLPHIN_N[57]);
    f.normal(DOLPHIN_N[65]);
    f.poly(&[p[56], p[57], p[65]]);
    f.normal(DOLPHIN_N[56]);
    f.normal(DOLPHIN_N[65]);
    f.normal(DOLPHIN_N[6]);
    f.poly(&[p[56], p[65], p[6]]);
    f.normal(DOLPHIN_N[56]);
    f.normal(DOLPHIN_N[6]);
    f.normal(DOLPHIN_N[63]);
    f.poly(&[p[56], p[6], p[63]]);
    f.normal(DOLPHIN_N[56]);
    f.normal(DOLPHIN_N[63]);
    f.normal(DOLPHIN_N[55]);
    f.poly(&[p[56], p[63], p[55]]);
    f.normal(DOLPHIN_N[54]);
    f.normal(DOLPHIN_N[62]);
    f.normal(DOLPHIN_N[5]);
    f.poly(&[p[54], p[62], p[5]]);
    f.normal(DOLPHIN_N[54]);
    f.normal(DOLPHIN_N[5]);
    f.normal(DOLPHIN_N[53]);
    f.poly(&[p[54], p[5], p[53]]);
    f.normal(DOLPHIN_N[52]);
    f.normal(DOLPHIN_N[53]);
    f.normal(DOLPHIN_N[5]);
    f.normal(DOLPHIN_N[60]);
    f.poly(&[p[52], p[53], p[5], p[60]]);
}

pub fn dolphin013(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[116]);
    f.normal(DOLPHIN_N[117]);
    f.normal(DOLPHIN_N[112]);
    f.normal(DOLPHIN_N[113]);
    f.poly(&[p[116], p[117], p[112], p[113]]);
    f.normal(DOLPHIN_N[114]);
    f.normal(DOLPHIN_N[113]);
    f.normal(DOLPHIN_N[112]);
    f.normal(DOLPHIN_N[115]);
    f.poly(&[p[114], p[113], p[112], p[115]]);
    f.normal(DOLPHIN_N[114]);
    f.normal(DOLPHIN_N[116]);
    f.normal(DOLPHIN_N[113]);
    f.poly(&[p[114], p[116], p[113]]);
    f.normal(DOLPHIN_N[114]);
    f.normal(DOLPHIN_N[7]);
    f.normal(DOLPHIN_N[116]);
    f.poly(&[p[114], p[7], p[116]]);
    f.normal(DOLPHIN_N[7]);
    f.normal(DOLPHIN_N[2]);
    f.normal(DOLPHIN_N[116]);
    f.poly(&[p[7], p[2], p[116]]);
    f.poly(&[p[2], p[7], p[8], p[99]]);
    f.poly(&[p[7], p[114], p[115], p[8]]);
    f.normal(DOLPHIN_N[117]);
    f.normal(DOLPHIN_N[99]);
    f.normal(DOLPHIN_N[8]);
    f.poly(&[p[117], p[99], p[8]]);
    f.normal(DOLPHIN_N[117]);
    f.normal(DOLPHIN_N[8]);
    f.normal(DOLPHIN_N[112]);
    f.poly(&[p[117], p[8], p[112]]);
    f.normal(DOLPHIN_N[112]);
    f.normal(DOLPHIN_N[8]);
    f.normal(DOLPHIN_N[115]);
    f.poly(&[p[112], p[8], p[115]]);
}

pub fn dolphin014(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[111]);
    f.normal(DOLPHIN_N[110]);
    f.normal(DOLPHIN_N[102]);
    f.normal(DOLPHIN_N[121]);
    f.poly(&[p[111], p[110], p[102], p[121]]);
    f.normal(DOLPHIN_N[111]);
    f.normal(DOLPHIN_N[97]);
    f.normal(DOLPHIN_N[110]);
    f.poly(&[p[111], p[97], p[110]]);
    f.normal(DOLPHIN_N[97]);
    f.normal(DOLPHIN_N[119]);
    f.normal(DOLPHIN_N[110]);
    f.poly(&[p[97], p[119], p[110]]);
    f.normal(DOLPHIN_N[97]);
    f.normal(DOLPHIN_N[99]);
    f.normal(DOLPHIN_N[119]);
    f.poly(&[p[97], p[99], p[119]]);
    f.normal(DOLPHIN_N[99]);
    f.normal(DOLPHIN_N[65]);
    f.normal(DOLPHIN_N[119]);
    f.poly(&[p[99], p[65], p[119]]);
    f.normal(DOLPHIN_N[65]);
    f.normal(DOLPHIN_N[66]);
    f.normal(DOLPHIN_N[119]);
    f.poly(&[p[65], p[66], p[119]]);
    f.poly(&[p[98], p[97], p[111], p[121]]);
    f.poly(&[p[2], p[99], p[97], p[98]]);
    f.normal(DOLPHIN_N[110]);
    f.normal(DOLPHIN_N[119]);
    f.normal(DOLPHIN_N[118]);
    f.normal(DOLPHIN_N[102]);
    f.poly(&[p[110], p[119], p[118], p[102]]);
    f.normal(DOLPHIN_N[119]);
    f.normal(DOLPHIN_N[66]);
    f.normal(DOLPHIN_N[67]);
    f.normal(DOLPHIN_N[118]);
    f.poly(&[p[119], p[66], p[67], p[118]]);
    f.normal(DOLPHIN_N[67]);
    f.normal(DOLPHIN_N[60]);
    f.normal(DOLPHIN_N[2]);
    f.poly(&[p[67], p[60], p[2]]);
    f.normal(DOLPHIN_N[67]);
    f.normal(DOLPHIN_N[2]);
    f.normal(DOLPHIN_N[118]);
    f.poly(&[p[67], p[2], p[118]]);
    f.normal(DOLPHIN_N[118]);
    f.normal(DOLPHIN_N[2]);
    f.normal(DOLPHIN_N[98]);
    f.poly(&[p[118], p[2], p[98]]);
    f.normal(DOLPHIN_N[118]);
    f.normal(DOLPHIN_N[98]);
    f.normal(DOLPHIN_N[102]);
    f.poly(&[p[118], p[98], p[102]]);
    f.normal(DOLPHIN_N[102]);
    f.normal(DOLPHIN_N[98]);
    f.normal(DOLPHIN_N[121]);
    f.poly(&[p[102], p[98], p[121]]);
}

pub fn dolphin015(f: &mut Fish, p: &Pts) {
    f.normal(DOLPHIN_N[55]);
    f.normal(DOLPHIN_N[3]);
    f.normal(DOLPHIN_N[54]);
    f.poly(&[p[55], p[3], p[54]]);
    f.normal(DOLPHIN_N[3]);
    f.normal(DOLPHIN_N[55]);
    f.normal(DOLPHIN_N[63]);
    f.poly(&[p[3], p[55], p[63]]);
    f.normal(DOLPHIN_N[3]);
    f.normal(DOLPHIN_N[63]);
    f.normal(DOLPHIN_N[100]);
    f.poly(&[p[3], p[63], p[100]]);
    f.normal(DOLPHIN_N[3]);
    f.normal(DOLPHIN_N[100]);
    f.normal(DOLPHIN_N[54]);
    f.poly(&[p[3], p[100], p[54]]);
    f.normal(DOLPHIN_N[54]);
    f.normal(DOLPHIN_N[100]);
    f.normal(DOLPHIN_N[62]);
    f.poly(&[p[54], p[100], p[62]]);
    f.normal(DOLPHIN_N[100]);
    f.normal(DOLPHIN_N[64]);
    f.normal(DOLPHIN_N[120]);
    f.poly(&[p[100], p[64], p[120]]);
    f.normal(DOLPHIN_N[100]);
    f.normal(DOLPHIN_N[63]);
    f.normal(DOLPHIN_N[64]);
    f.poly(&[p[100], p[63], p[64]]);
    f.normal(DOLPHIN_N[63]);
    f.normal(DOLPHIN_N[6]);
    f.normal(DOLPHIN_N[64]);
    f.poly(&[p[63], p[6], p[64]]);
    f.normal(DOLPHIN_N[64]);
    f.normal(DOLPHIN_N[6]);
    f.normal(DOLPHIN_N[99]);
    f.poly(&[p[64], p[6], p[99]]);
    f.normal(DOLPHIN_N[64]);
    f.normal(DOLPHIN_N[99]);
    f.normal(DOLPHIN_N[117]);
    f.poly(&[p[64], p[99], p[117]]);
    f.normal(DOLPHIN_N[120]);
    f.normal(DOLPHIN_N[64]);
    f.normal(DOLPHIN_N[117]);
    f.normal(DOLPHIN_N[116]);
    f.poly(&[p[120], p[64], p[117], p[116]]);
    f.normal(DOLPHIN_N[6]);
    f.normal(DOLPHIN_N[65]);
    f.normal(DOLPHIN_N[99]);
    f.poly(&[p[6], p[65], p[99]]);
    f.normal(DOLPHIN_N[62]);
    f.normal(DOLPHIN_N[100]);
    f.normal(DOLPHIN_N[120]);
    f.poly(&[p[62], p[100], p[120]]);
    f.normal(DOLPHIN_N[5]);
    f.normal(DOLPHIN_N[62]);
    f.normal(DOLPHIN_N[120]);
    f.poly(&[p[5], p[62], p[120]]);
    f.normal(DOLPHIN_N[5]);
    f.normal(DOLPHIN_N[120]);
    f.normal(DOLPHIN_N[2]);
    f.poly(&[p[5], p[120], p[2]]);
    f.normal(DOLPHIN_N[2]);
    f.normal(DOLPHIN_N[120]);
    f.normal(DOLPHIN_N[116]);
    f.poly(&[p[2], p[120], p[116]]);
    f.normal(DOLPHIN_N[60]);
    f.normal(DOLPHIN_N[5]);
    f.normal(DOLPHIN_N[2]);
    f.poly(&[p[60], p[5], p[2]]);
}

pub fn dolphin016(f: &mut Fish, p: &Pts) {
    f.depth_test(false);
    f.poly(&[p[123], p[124], p[125], p[126], p[127], p[128]]);
    f.poly(&[p[129], p[130], p[131], p[132], p[133], p[134]]);
    f.poly(&[p[103], p[105], p[108]]);
    f.depth_test(true);
}

/// `shark`'s face normals, upstream's `N001` and friends.
#[rustfmt::skip]
pub const SHARK_N: [[f32; 3]; 83] = [
    [0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0], // N001
    [0.000077, -0.020611, 0.999788], // N002
    [0.961425, 0.258729, -0.09339], // N003
    [0.510811, -0.769633, -0.383063], // N004
    [0.400123, 0.855734, -0.328055], // N005
    [-0.770715, 0.610204, -0.18344], // N006
    [-0.915597, -0.373345, -0.149316], // N007
    [-0.972788, 0.208921, -0.100179], // N008
    [-0.939713, -0.312268, -0.139383], // N009
    [-0.624138, -0.741047, -0.247589], // N010
    [0.591434, -0.768401, -0.244471], // N011
    [0.935152, -0.328495, -0.132598], // N012
    [0.997102, 0.074243, -0.016593], // N013
    [0.969995, 0.241712, -0.026186], // N014
    [0.844539, 0.502628, -0.184714], // N015
    [-0.906608, 0.386308, -0.169787], // N016
    [-0.970016, 0.241698, -0.025516], // N017
    [-0.998652, 0.050493, -0.012045], // N018
    [-0.942685, -0.333051, -0.020556], // N019
    [-0.660944, -0.750276, 0.01548], // N020
    [0.503549, -0.862908, -0.042749], // N021
    [0.953202, -0.302092, -0.012089], // N022
    [0.998738, 0.023574, 0.044344], // N023
    [0.979297, 0.193272, 0.060202], // N024
    [0.7983, 0.464885, 0.382883], // N025
    [-0.75659, 0.452403, 0.472126], // N026
    [-0.953855, 0.293003, 0.065651], // N027
    [-0.998033, 0.040292, 0.048028], // N028
    [-0.977079, -0.204288, 0.059858], // N029
    [-0.729117, -0.675304, 0.11114], // N030
    [0.598361, -0.792753, 0.116221], // N031
    [0.965192, -0.252991, 0.066332], // N032
    [0.998201, -0.00279, 0.059892], // N033
    [0.978657, 0.193135, 0.070207], // N034
    [0.718815, 0.680392, 0.142733], // N035
    [-0.383096, 0.906212, 0.178936], // N036
    [-0.952831, 0.29259, 0.080647], // N037
    [-0.99768, 0.032417, 0.059861], // N038
    [-0.982629, -0.169881, 0.0747], // N039
    [-0.695424, -0.703466, 0.1467], // N040
    [0.359323, -0.915531, 0.180805], // N041
    [0.943356, -0.319387, 0.089842], // N042
    [0.998272, -0.032435, 0.048993], // N043
    [0.978997, 0.193205, 0.065084], // N044
    [0.872144, 0.470094, -0.135565], // N045
    [-0.664282, 0.737945, -0.119027], // N046
    [-0.954508, 0.28857, 0.075107], // N047
    [-0.998273, 0.032406, 0.048993], // N048
    [-0.979908, -0.193579, 0.048038], // N049
    [-0.858736, -0.507202, -0.072938], // N050
    [0.643545, -0.763887, -0.048237], // N051
    [0.95558, -0.288954, 0.058068], // N052
    [0.0, 1.0, 0.0], // N053
    [0.0, 1.0, 0.0], // N054
    [0.0, 1.0, 0.0], // N055
    [0.0, 1.0, 0.0], // N056
    [0.0, 1.0, 0.0], // N057
    [0.00005, 0.793007, -0.609213], // N058
    [0.91351, 0.235418, -0.331779], // N059
    [-0.80797, 0.495, -0.319625], // N060
    [0.0, 0.784687, -0.619892], // N061
    [0.0, -1.0, 0.0], // N062
    [0.0, 1.0, 0.0], // N063
    [0.0, 1.0, 0.0], // N064
    [0.0, 1.0, 0.0], // N065
    [-0.055784, 0.257059, 0.964784], // N066
    [0.0, 1.0, 0.0], // N067
    [0.0, 1.0, 0.0], // N068
    [-0.000505, -0.929775, -0.368127], // N069
    [0.0, 1.0, 0.0], // N070
    [-0.987102, 0.131723, -0.090984], // N071
    [-0.987102, 0.131723, -0.090984], // N072
    [-0.987102, 0.131723, -0.090984], // N073
    [0.0, 1.0, 0.0], // N074
    [0.0, 1.0, 0.0], // N075
    [0.0, 1.0, 0.0], // N076
    [0.99521, 0.071962, -0.066168], // N077
    [0.99521, 0.071962, -0.066168], // N078
    [0.99521, 0.071962, -0.066168], // N079
    [0.0, 1.0, 0.0], // N080
    [0.0, 1.0, 0.0], // N081
    [0.0, 1.0, 0.0], // N082
];

/// `shark`'s points at rest, which are upstream's `P001` and
/// its immutable copy `iP001` both: the two are declared with the same
/// values and only the working copy is ever written to.
#[rustfmt::skip]
pub const SHARK_P: [[f32; 3]; 83] = [
    [0.0, 0.0, 0.0],
    [0.0, 0.0, 0.0], // P001
    [0.0, -36.59, 5687.72], // P002
    [90.0, 114.73, 724.38], // P003
    [58.24, -146.84, 262.35], // P004
    [27.81, 231.52, 510.43], // P005
    [-27.81, 230.43, 509.76], // P006
    [-46.09, -146.83, 265.84], // P007
    [-90.0, 103.84, 718.53], // P008
    [-131.1, -165.92, 834.85], // P009
    [-27.81, -285.31, 500.0], // P010
    [27.81, -285.32, 500.0], // P011
    [147.96, -170.89, 845.5], // P012
    [180.0, 0.0, 2000.0], // P013
    [145.62, 352.67, 2000.0], // P014
    [55.62, 570.63, 2000.0], // P015
    [-55.62, 570.64, 2000.0], // P016
    [-145.62, 352.68, 2000.0], // P017
    [-180.0, 0.01, 2000.0], // P018
    [-178.2, -352.66, 2001.61], // P019
    [-55.63, -570.63, 2000.0], // P020
    [55.62, -570.64, 2000.0], // P021
    [179.91, -352.69, 1998.39], // P022
    [150.0, 0.0, 3000.0], // P023
    [121.35, 293.89, 3000.0], // P024
    [46.35, 502.93, 2883.09], // P025
    [-46.35, 497.45, 2877.24], // P026
    [-121.35, 293.9, 3000.0], // P027
    [-150.0, 0.0, 3000.0], // P028
    [-152.21, -304.84, 2858.68], // P029
    [-46.36, -475.52, 3000.0], // P030
    [46.35, -475.53, 3000.0], // P031
    [155.64, -304.87, 2863.5], // P032
    [90.0, 0.0, 4000.0], // P033
    [72.81, 176.33, 4000.0], // P034
    [27.81, 285.32, 4000.0], // P035
    [-27.81, 285.32, 4000.0], // P036
    [-72.81, 176.34, 4000.0], // P037
    [-90.0, 0.0, 4000.0], // P038
    [-72.81, -176.33, 4000.0], // P039
    [-27.81, -285.31, 4000.0], // P040
    [27.81, -285.32, 4000.0], // P041
    [72.81, -176.34, 4000.0], // P042
    [30.0, 0.0, 5000.0], // P043
    [24.27, 58.78, 5000.0], // P044
    [9.27, 95.11, 5000.0], // P045
    [-9.27, 95.11, 5000.0], // P046
    [-24.27, 58.78, 5000.0], // P047
    [-30.0, 0.0, 5000.0], // P048
    [-24.27, -58.78, 5000.0], // P049
    [-9.27, -95.1, 5000.0], // P050
    [9.27, -95.11, 5000.0], // P051
    [24.27, -58.78, 5000.0], // P052
    [0.0, 0.0, 0.0], // P053
    [0.0, 0.0, 0.0], // P054
    [0.0, 0.0, 0.0], // P055
    [0.0, 0.0, 0.0], // P056
    [0.0, 0.0, 0.0], // P057
    [0.0, 1212.72, 2703.08], // P058
    [50.36, 0.0, 108.14], // P059
    [-22.18, 0.0, 108.14], // P060
    [0.0, 1181.61, 6344.65], // P061
    [516.45, -887.08, 2535.45], // P062
    [-545.69, -879.31, 2555.63], // P063
    [618.89, -1005.64, 2988.32], // P064
    [-635.37, -1014.79, 2938.68], // P065
    [0.0, 1374.43, 3064.18], // P066
    [158.49, -11.89, 1401.56], // P067
    [-132.08, -17.9, 1394.31], // P068
    [0.0, -418.25, 5765.04], // P069
    [0.0, 1266.91, 6629.6], // P070
    [-139.12, -124.96, 997.98], // P071
    [-139.24, -110.18, 1020.68], // P072
    [-137.33, -94.52, 1022.63], // P073
    [-137.03, -79.91, 996.89], // P074
    [-135.21, -91.48, 969.14], // P075
    [-135.39, -110.87, 968.76], // P076
    [150.23, -78.44, 995.53], // P077
    [152.79, -92.76, 1018.46], // P078
    [154.19, -110.2, 1020.55], // P079
    [151.33, -124.15, 993.77], // P080
    [150.49, -111.19, 969.86], // P081
    [150.79, -92.41, 969.7], // P082
];

pub fn fish001(f: &mut Fish, p: &Pts) {
    f.normal(SHARK_N[5]);
    f.normal(SHARK_N[59]);
    f.normal(SHARK_N[60]);
    f.normal(SHARK_N[6]);
    f.poly(&[p[5], p[59], p[60], p[6]]);
    f.normal(SHARK_N[15]);
    f.normal(SHARK_N[5]);
    f.normal(SHARK_N[6]);
    f.normal(SHARK_N[16]);
    f.poly(&[p[15], p[5], p[6], p[16]]);
    f.normal(SHARK_N[6]);
    f.normal(SHARK_N[60]);
    f.normal(SHARK_N[8]);
    f.poly(&[p[6], p[60], p[8]]);
    f.normal(SHARK_N[16]);
    f.normal(SHARK_N[6]);
    f.normal(SHARK_N[8]);
    f.poly(&[p[16], p[6], p[8]]);
    f.normal(SHARK_N[16]);
    f.normal(SHARK_N[8]);
    f.normal(SHARK_N[17]);
    f.poly(&[p[16], p[8], p[17]]);
    f.normal(SHARK_N[17]);
    f.normal(SHARK_N[8]);
    f.normal(SHARK_N[18]);
    f.poly(&[p[17], p[8], p[18]]);
    f.normal(SHARK_N[8]);
    f.normal(SHARK_N[9]);
    f.normal(SHARK_N[18]);
    f.poly(&[p[8], p[9], p[18]]);
    f.normal(SHARK_N[8]);
    f.normal(SHARK_N[60]);
    f.normal(SHARK_N[9]);
    f.poly(&[p[8], p[60], p[9]]);
    f.normal(SHARK_N[7]);
    f.normal(SHARK_N[10]);
    f.normal(SHARK_N[9]);
    f.poly(&[p[7], p[10], p[9]]);
    f.normal(SHARK_N[9]);
    f.normal(SHARK_N[19]);
    f.normal(SHARK_N[18]);
    f.poly(&[p[9], p[19], p[18]]);
    f.normal(SHARK_N[9]);
    f.normal(SHARK_N[10]);
    f.normal(SHARK_N[19]);
    f.poly(&[p[9], p[10], p[19]]);
    f.normal(SHARK_N[10]);
    f.normal(SHARK_N[20]);
    f.normal(SHARK_N[19]);
    f.poly(&[p[10], p[20], p[19]]);
    f.normal(SHARK_N[10]);
    f.normal(SHARK_N[11]);
    f.normal(SHARK_N[21]);
    f.normal(SHARK_N[20]);
    f.poly(&[p[10], p[11], p[21], p[20]]);
    f.normal(SHARK_N[4]);
    f.normal(SHARK_N[11]);
    f.normal(SHARK_N[10]);
    f.normal(SHARK_N[7]);
    f.poly(&[p[4], p[11], p[10], p[7]]);
    f.normal(SHARK_N[4]);
    f.normal(SHARK_N[12]);
    f.normal(SHARK_N[11]);
    f.poly(&[p[4], p[12], p[11]]);
    f.normal(SHARK_N[12]);
    f.normal(SHARK_N[22]);
    f.normal(SHARK_N[11]);
    f.poly(&[p[12], p[22], p[11]]);
    f.normal(SHARK_N[11]);
    f.normal(SHARK_N[22]);
    f.normal(SHARK_N[21]);
    f.poly(&[p[11], p[22], p[21]]);
    f.normal(SHARK_N[59]);
    f.normal(SHARK_N[5]);
    f.normal(SHARK_N[15]);
    f.poly(&[p[59], p[5], p[15]]);
    f.normal(SHARK_N[15]);
    f.normal(SHARK_N[14]);
    f.normal(SHARK_N[3]);
    f.poly(&[p[15], p[14], p[3]]);
    f.normal(SHARK_N[15]);
    f.normal(SHARK_N[3]);
    f.normal(SHARK_N[59]);
    f.poly(&[p[15], p[3], p[59]]);
    f.normal(SHARK_N[14]);
    f.normal(SHARK_N[13]);
    f.normal(SHARK_N[3]);
    f.poly(&[p[14], p[13], p[3]]);
    f.normal(SHARK_N[3]);
    f.normal(SHARK_N[12]);
    f.normal(SHARK_N[59]);
    f.poly(&[p[3], p[12], p[59]]);
    f.normal(SHARK_N[13]);
    f.normal(SHARK_N[12]);
    f.normal(SHARK_N[3]);
    f.poly(&[p[13], p[12], p[3]]);
    f.normal(SHARK_N[13]);
    f.normal(SHARK_N[22]);
    f.normal(SHARK_N[12]);
    f.poly(&[p[13], p[22], p[12]]);
    f.poly(&[p[71], p[72], p[73], p[74], p[75], p[76]]);
    f.poly(&[p[77], p[78], p[79], p[80], p[81], p[82]]);
}

pub fn fish002(f: &mut Fish, p: &Pts) {
    f.normal(SHARK_N[13]);
    f.normal(SHARK_N[14]);
    f.normal(SHARK_N[24]);
    f.normal(SHARK_N[23]);
    f.poly(&[p[13], p[14], p[24], p[23]]);
    f.normal(SHARK_N[14]);
    f.normal(SHARK_N[15]);
    f.normal(SHARK_N[25]);
    f.normal(SHARK_N[24]);
    f.poly(&[p[14], p[15], p[25], p[24]]);
    f.normal(SHARK_N[16]);
    f.normal(SHARK_N[17]);
    f.normal(SHARK_N[27]);
    f.normal(SHARK_N[26]);
    f.poly(&[p[16], p[17], p[27], p[26]]);
    f.normal(SHARK_N[17]);
    f.normal(SHARK_N[18]);
    f.normal(SHARK_N[28]);
    f.normal(SHARK_N[27]);
    f.poly(&[p[17], p[18], p[28], p[27]]);
    f.normal(SHARK_N[20]);
    f.normal(SHARK_N[21]);
    f.normal(SHARK_N[31]);
    f.normal(SHARK_N[30]);
    f.poly(&[p[20], p[21], p[31], p[30]]);
    f.normal(SHARK_N[13]);
    f.normal(SHARK_N[23]);
    f.normal(SHARK_N[22]);
    f.poly(&[p[13], p[23], p[22]]);
    f.normal(SHARK_N[22]);
    f.normal(SHARK_N[23]);
    f.normal(SHARK_N[32]);
    f.poly(&[p[22], p[23], p[32]]);
    f.normal(SHARK_N[22]);
    f.normal(SHARK_N[32]);
    f.normal(SHARK_N[31]);
    f.poly(&[p[22], p[32], p[31]]);
    f.normal(SHARK_N[22]);
    f.normal(SHARK_N[31]);
    f.normal(SHARK_N[21]);
    f.poly(&[p[22], p[31], p[21]]);
    f.normal(SHARK_N[18]);
    f.normal(SHARK_N[19]);
    f.normal(SHARK_N[29]);
    f.poly(&[p[18], p[19], p[29]]);
    f.normal(SHARK_N[18]);
    f.normal(SHARK_N[29]);
    f.normal(SHARK_N[28]);
    f.poly(&[p[18], p[29], p[28]]);
    f.normal(SHARK_N[19]);
    f.normal(SHARK_N[20]);
    f.normal(SHARK_N[30]);
    f.poly(&[p[19], p[20], p[30]]);
    f.normal(SHARK_N[19]);
    f.normal(SHARK_N[30]);
    f.normal(SHARK_N[29]);
    f.poly(&[p[19], p[30], p[29]]);
}

pub fn fish003(f: &mut Fish, p: &Pts) {
    f.normal(SHARK_N[32]);
    f.normal(SHARK_N[23]);
    f.normal(SHARK_N[33]);
    f.normal(SHARK_N[42]);
    f.poly(&[p[32], p[23], p[33], p[42]]);
    f.normal(SHARK_N[31]);
    f.normal(SHARK_N[32]);
    f.normal(SHARK_N[42]);
    f.normal(SHARK_N[41]);
    f.poly(&[p[31], p[32], p[42], p[41]]);
    f.normal(SHARK_N[23]);
    f.normal(SHARK_N[24]);
    f.normal(SHARK_N[34]);
    f.normal(SHARK_N[33]);
    f.poly(&[p[23], p[24], p[34], p[33]]);
    f.normal(SHARK_N[24]);
    f.normal(SHARK_N[25]);
    f.normal(SHARK_N[35]);
    f.normal(SHARK_N[34]);
    f.poly(&[p[24], p[25], p[35], p[34]]);
    f.normal(SHARK_N[30]);
    f.normal(SHARK_N[31]);
    f.normal(SHARK_N[41]);
    f.normal(SHARK_N[40]);
    f.poly(&[p[30], p[31], p[41], p[40]]);
    f.normal(SHARK_N[25]);
    f.normal(SHARK_N[26]);
    f.normal(SHARK_N[36]);
    f.normal(SHARK_N[35]);
    f.poly(&[p[25], p[26], p[36], p[35]]);
    f.normal(SHARK_N[26]);
    f.normal(SHARK_N[27]);
    f.normal(SHARK_N[37]);
    f.normal(SHARK_N[36]);
    f.poly(&[p[26], p[27], p[37], p[36]]);
    f.normal(SHARK_N[27]);
    f.normal(SHARK_N[28]);
    f.normal(SHARK_N[38]);
    f.normal(SHARK_N[37]);
    f.poly(&[p[27], p[28], p[38], p[37]]);
    f.normal(SHARK_N[28]);
    f.normal(SHARK_N[29]);
    f.normal(SHARK_N[39]);
    f.normal(SHARK_N[38]);
    f.poly(&[p[28], p[29], p[39], p[38]]);
    f.normal(SHARK_N[29]);
    f.normal(SHARK_N[30]);
    f.normal(SHARK_N[40]);
    f.normal(SHARK_N[39]);
    f.poly(&[p[29], p[30], p[40], p[39]]);
}

pub fn fish004(f: &mut Fish, p: &Pts) {
    f.normal(SHARK_N[40]);
    f.normal(SHARK_N[41]);
    f.normal(SHARK_N[51]);
    f.normal(SHARK_N[50]);
    f.poly(&[p[40], p[41], p[51], p[50]]);
    f.normal(SHARK_N[41]);
    f.normal(SHARK_N[42]);
    f.normal(SHARK_N[52]);
    f.normal(SHARK_N[51]);
    f.poly(&[p[41], p[42], p[52], p[51]]);
    f.normal(SHARK_N[42]);
    f.normal(SHARK_N[33]);
    f.normal(SHARK_N[43]);
    f.normal(SHARK_N[52]);
    f.poly(&[p[42], p[33], p[43], p[52]]);
    f.normal(SHARK_N[33]);
    f.normal(SHARK_N[34]);
    f.normal(SHARK_N[44]);
    f.normal(SHARK_N[43]);
    f.poly(&[p[33], p[34], p[44], p[43]]);
    f.normal(SHARK_N[34]);
    f.normal(SHARK_N[35]);
    f.normal(SHARK_N[45]);
    f.normal(SHARK_N[44]);
    f.poly(&[p[34], p[35], p[45], p[44]]);
    f.normal(SHARK_N[35]);
    f.normal(SHARK_N[36]);
    f.normal(SHARK_N[46]);
    f.normal(SHARK_N[45]);
    f.poly(&[p[35], p[36], p[46], p[45]]);
    f.normal(SHARK_N[36]);
    f.normal(SHARK_N[37]);
    f.normal(SHARK_N[47]);
    f.normal(SHARK_N[46]);
    f.poly(&[p[36], p[37], p[47], p[46]]);
    f.normal(SHARK_N[37]);
    f.normal(SHARK_N[38]);
    f.normal(SHARK_N[48]);
    f.normal(SHARK_N[47]);
    f.poly(&[p[37], p[38], p[48], p[47]]);
    f.normal(SHARK_N[38]);
    f.normal(SHARK_N[39]);
    f.normal(SHARK_N[49]);
    f.normal(SHARK_N[48]);
    f.poly(&[p[38], p[39], p[49], p[48]]);
    f.normal(SHARK_N[39]);
    f.normal(SHARK_N[40]);
    f.normal(SHARK_N[50]);
    f.normal(SHARK_N[49]);
    f.poly(&[p[39], p[40], p[50], p[49]]);
    f.normal(SHARK_N[70]);
    f.normal(SHARK_N[61]);
    f.normal(SHARK_N[2]);
    f.poly(&[p[70], p[61], p[2]]);
    f.normal(SHARK_N[61]);
    f.normal(SHARK_N[46]);
    f.normal(SHARK_N[2]);
    f.poly(&[p[61], p[46], p[2]]);
    f.normal(SHARK_N[45]);
    f.normal(SHARK_N[46]);
    f.normal(SHARK_N[61]);
    f.poly(&[p[45], p[46], p[61]]);
    f.normal(SHARK_N[2]);
    f.normal(SHARK_N[61]);
    f.normal(SHARK_N[70]);
    f.poly(&[p[2], p[61], p[70]]);
    f.normal(SHARK_N[2]);
    f.normal(SHARK_N[45]);
    f.normal(SHARK_N[61]);
    f.poly(&[p[2], p[45], p[61]]);
}

pub fn fish005(f: &mut Fish, p: &Pts) {
    f.normal(SHARK_N[2]);
    f.normal(SHARK_N[44]);
    f.normal(SHARK_N[45]);
    f.poly(&[p[2], p[44], p[45]]);
    f.normal(SHARK_N[2]);
    f.normal(SHARK_N[43]);
    f.normal(SHARK_N[44]);
    f.poly(&[p[2], p[43], p[44]]);
    f.normal(SHARK_N[2]);
    f.normal(SHARK_N[52]);
    f.normal(SHARK_N[43]);
    f.poly(&[p[2], p[52], p[43]]);
    f.normal(SHARK_N[2]);
    f.normal(SHARK_N[51]);
    f.normal(SHARK_N[52]);
    f.poly(&[p[2], p[51], p[52]]);
    f.normal(SHARK_N[2]);
    f.normal(SHARK_N[46]);
    f.normal(SHARK_N[47]);
    f.poly(&[p[2], p[46], p[47]]);
    f.normal(SHARK_N[2]);
    f.normal(SHARK_N[47]);
    f.normal(SHARK_N[48]);
    f.poly(&[p[2], p[47], p[48]]);
    f.normal(SHARK_N[2]);
    f.normal(SHARK_N[48]);
    f.normal(SHARK_N[49]);
    f.poly(&[p[2], p[48], p[49]]);
    f.normal(SHARK_N[2]);
    f.normal(SHARK_N[49]);
    f.normal(SHARK_N[50]);
    f.poly(&[p[2], p[49], p[50]]);
    f.normal(SHARK_N[50]);
    f.normal(SHARK_N[51]);
    f.normal(SHARK_N[69]);
    f.poly(&[p[50], p[51], p[69]]);
    f.normal(SHARK_N[51]);
    f.normal(SHARK_N[2]);
    f.normal(SHARK_N[69]);
    f.poly(&[p[51], p[2], p[69]]);
    f.normal(SHARK_N[50]);
    f.normal(SHARK_N[69]);
    f.normal(SHARK_N[2]);
    f.poly(&[p[50], p[69], p[2]]);
}

pub fn fish006(f: &mut Fish, p: &Pts) {
    f.normal(SHARK_N[66]);
    f.normal(SHARK_N[16]);
    f.normal(SHARK_N[26]);
    f.poly(&[p[66], p[16], p[26]]);
    f.normal(SHARK_N[15]);
    f.normal(SHARK_N[66]);
    f.normal(SHARK_N[25]);
    f.poly(&[p[15], p[66], p[25]]);
    f.normal(SHARK_N[25]);
    f.normal(SHARK_N[66]);
    f.normal(SHARK_N[26]);
    f.poly(&[p[25], p[66], p[26]]);
    f.normal(SHARK_N[66]);
    f.normal(SHARK_N[58]);
    f.normal(SHARK_N[16]);
    f.poly(&[p[66], p[58], p[16]]);
    f.normal(SHARK_N[15]);
    f.normal(SHARK_N[58]);
    f.normal(SHARK_N[66]);
    f.poly(&[p[15], p[58], p[66]]);
    f.normal(SHARK_N[58]);
    f.normal(SHARK_N[15]);
    f.normal(SHARK_N[16]);
    f.poly(&[p[58], p[15], p[16]]);
}

pub fn fish007(f: &mut Fish, p: &Pts) {
    f.normal(SHARK_N[62]);
    f.normal(SHARK_N[22]);
    f.normal(SHARK_N[32]);
    f.poly(&[p[62], p[22], p[32]]);
    f.normal(SHARK_N[62]);
    f.normal(SHARK_N[32]);
    f.normal(SHARK_N[64]);
    f.poly(&[p[62], p[32], p[64]]);
    f.normal(SHARK_N[22]);
    f.normal(SHARK_N[62]);
    f.normal(SHARK_N[32]);
    f.poly(&[p[22], p[62], p[32]]);
    f.normal(SHARK_N[62]);
    f.normal(SHARK_N[64]);
    f.normal(SHARK_N[32]);
    f.poly(&[p[62], p[64], p[32]]);
}

pub fn fish008(f: &mut Fish, p: &Pts) {
    f.normal(SHARK_N[63]);
    f.normal(SHARK_N[19]);
    f.normal(SHARK_N[29]);
    f.poly(&[p[63], p[19], p[29]]);
    f.normal(SHARK_N[19]);
    f.normal(SHARK_N[63]);
    f.normal(SHARK_N[29]);
    f.poly(&[p[19], p[63], p[29]]);
    f.normal(SHARK_N[63]);
    f.normal(SHARK_N[29]);
    f.normal(SHARK_N[65]);
    f.poly(&[p[63], p[29], p[65]]);
    f.normal(SHARK_N[63]);
    f.normal(SHARK_N[65]);
    f.normal(SHARK_N[29]);
    f.poly(&[p[63], p[65], p[29]]);
}

pub fn fish009(f: &mut Fish, p: &Pts) {
    f.poly(&[p[59], p[12], p[9], p[60]]);
    f.poly(&[p[12], p[4], p[7], p[9]]);
}

pub fn fish1(f: &mut Fish, p: &Pts) {
    fish004(f, p);
    fish005(f, p);
    fish003(f, p);
    fish007(f, p);
    fish006(f, p);
    fish002(f, p);
    fish008(f, p);
    fish009(f, p);
    fish001(f, p);
}

pub fn fish2(f: &mut Fish, p: &Pts) {
    fish005(f, p);
    fish004(f, p);
    fish003(f, p);
    fish008(f, p);
    fish006(f, p);
    fish002(f, p);
    fish007(f, p);
    fish009(f, p);
    fish001(f, p);
}

pub fn fish3(f: &mut Fish, p: &Pts) {
    fish005(f, p);
    fish004(f, p);
    fish007(f, p);
    fish003(f, p);
    fish002(f, p);
    fish008(f, p);
    fish009(f, p);
    fish001(f, p);
    fish006(f, p);
}

pub fn fish4(f: &mut Fish, p: &Pts) {
    fish005(f, p);
    fish004(f, p);
    fish008(f, p);
    fish003(f, p);
    fish002(f, p);
    fish007(f, p);
    fish009(f, p);
    fish001(f, p);
    fish006(f, p);
}

pub fn fish5(f: &mut Fish, p: &Pts) {
    fish009(f, p);
    fish006(f, p);
    fish007(f, p);
    fish001(f, p);
    fish002(f, p);
    fish003(f, p);
    fish008(f, p);
    fish004(f, p);
    fish005(f, p);
}

pub fn fish6(f: &mut Fish, p: &Pts) {
    fish009(f, p);
    fish006(f, p);
    fish008(f, p);
    fish001(f, p);
    fish002(f, p);
    fish007(f, p);
    fish003(f, p);
    fish004(f, p);
    fish005(f, p);
}

pub fn fish7(f: &mut Fish, p: &Pts) {
    fish009(f, p);
    fish001(f, p);
    fish007(f, p);
    fish005(f, p);
    fish002(f, p);
    fish008(f, p);
    fish003(f, p);
    fish004(f, p);
    fish006(f, p);
}

pub fn fish8(f: &mut Fish, p: &Pts) {
    fish009(f, p);
    fish008(f, p);
    fish001(f, p);
    fish002(f, p);
    fish007(f, p);
    fish003(f, p);
    fish005(f, p);
    fish004(f, p);
    fish006(f, p);
}

/// `whale`'s face normals, upstream's `N001` and friends.
#[rustfmt::skip]
pub const WHALE_N: [[f32; 3]; 122] = [
    [0.0, 0.0, 0.0],
    [0.019249, 0.01134, -0.99975], // N001
    [-0.132579, 0.954547, 0.266952], // N002
    [-0.196061, 0.980392, -0.019778], // N003
    [0.695461, 0.604704, 0.388158], // N004
    [0.8706, 0.425754, 0.246557], // N005
    [-0.881191, 0.392012, 0.264251], // N006
    [0.0, 1.0, 0.0], // N007
    [-0.341437, 0.887477, 0.309523], // N008
    [0.124035, -0.992278, 0.0], // N009
    [0.242536, 0.0, -0.970143], // N010
    [0.588172, 0.0, 0.808736], // N011
    [0.929824, -0.340623, -0.139298], // N012
    [0.954183, 0.267108, -0.134865], // N013
    [0.495127, 0.855436, -0.151914], // N014
    [-0.390199, 0.906569, -0.160867], // N015
    [-0.923605, 0.354581, -0.145692], // N016
    [-0.955796, -0.260667, -0.136036], // N017
    [-0.501283, -0.853462, -0.14254], // N018
    [0.4053, -0.901974, -0.148913], // N019
    [0.909913, -0.392746, -0.133451], // N020
    [0.936494, 0.331147, -0.115414], // N021
    [0.600131, 0.793724, -0.099222], // N022
    [-0.231556, 0.968361, -0.093053], // N023
    [-0.844369, 0.52533, -0.105211], // N024
    [-0.982725, -0.136329, -0.125164], // N025
    [-0.560844, -0.822654, -0.093241], // N026
    [0.263884, -0.959981, -0.093817], // N027
    [0.842057, -0.525192, -0.122938], // N028
    [0.92162, 0.367565, -0.124546], // N029
    [0.613927, 0.784109, -0.090918], // N030
    [-0.448754, 0.888261, -0.098037], // N031
    [-0.891865, 0.434376, -0.126077], // N032
    [-0.881447, -0.448017, -0.149437], // N033
    [-0.345647, -0.922057, -0.174183], // N034
    [0.307998, -0.941371, -0.137688], // N035
    [0.806316, -0.574647, -0.140124], // N036
    [0.961346, 0.233646, -0.145681], // N037
    [0.488451, 0.865586, -0.110351], // N038
    [-0.37429, 0.921953, -0.099553], // N039
    [-0.928504, 0.344533, -0.138485], // N040
    [-0.918419, -0.371792, -0.135189], // N041
    [-0.520666, -0.833704, -0.183968], // N042
    [0.339204, -0.920273, -0.195036], // N043
    [0.921475, -0.387382, -0.028636], // N044
    [0.842465, 0.533335, -0.076204], // N045
    [0.38011, 0.924939, 0.002073], // N046
    [-0.276128, 0.961073, -0.009579], // N047
    [-0.879684, 0.473001, -0.04925], // N048
    [-0.947184, -0.317614, -0.044321], // N049
    [-0.642059, -0.764933, -0.051363], // N050
    [0.466794, -0.880921, -0.07799], // N051
    [0.898509, -0.432277, 0.076279], // N052
    [0.938985, 0.328141, 0.103109], // N053
    [0.44242, 0.895745, 0.043647], // N054
    [-0.255163, 0.966723, 0.018407], // N055
    [-0.833769, 0.54065, 0.111924], // N056
    [-0.953653, -0.289939, 0.080507], // N057
    [-0.672357, -0.730524, 0.119461], // N058
    [0.522249, -0.846652, 0.102157], // N059
    [0.885868, -0.427631, 0.179914], // N060
    [0.0, 1.0, 0.0], // N061
    [0.648942, 0.743116, 0.163255], // N062
    [-0.578967, 0.80773, 0.111219], // N063
    [0.0, 1.0, 0.0], // N064
    [-0.909864, -0.352202, 0.219321], // N065
    [-0.502541, -0.81809, 0.27961], // N066
    [0.322919, -0.915358, 0.240504], // N067
    [0.242536, 0.0, -0.970143], // N068
    [0.0, 1.0, 0.0], // N069
    [0.0, 1.0, 0.0], // N070
    [0.0, 1.0, 0.0], // N071
    [0.0, 1.0, 0.0], // N072
    [0.0, 1.0, 0.0], // N073
    [0.0, 1.0, 0.0], // N074
    [0.03122, 0.999025, -0.03122], // N075
    [0.0, 1.0, 0.0], // N076
    [0.446821, 0.893642, 0.041889], // N077
    [0.863035, -0.10098, 0.494949], // N078
    [0.585597, -0.808215, 0.062174], // N079
    [0.0, 1.0, 0.0], // N080
    [1.0, 0.0, 0.0], // N081
    [0.0, 1.0, 0.0], // N082
    [-1.0, 0.0, 0.0], // N083
    [-0.478893, 0.837129, -0.264343], // N084
    [0.0, 1.0, 0.0], // N085
    [0.763909, 0.539455, -0.354163], // N086
    [0.446821, 0.893642, 0.041889], // N087
    [0.385134, -0.908288, 0.163352], // N088
    [-0.605952, 0.779253, -0.159961], // N089
    [0.0, 1.0, 0.0], // N090
    [0.0, 1.0, 0.0], // N091
    [0.0, 1.0, 0.0], // N092
    [0.0, 1.0, 0.0], // N093
    [1.0, 0.0, 0.0], // N094
    [-1.0, 0.0, 0.0], // N095
    [0.644444, -0.621516, 0.445433], // N096
    [-0.760896, -0.474416, 0.442681], // N097
    [0.636888, -0.464314, 0.615456], // N098
    [-0.710295, 0.647038, 0.277168], // N099
    [0.009604, 0.993655, 0.112063], // N100
    [0.0, 1.0, 0.0], // N101
    [0.0, 1.0, 0.0], // N102
    [0.0, 1.0, 0.0], // N103
    [0.031837, 0.999285, 0.020415], // N104
    [0.031837, 0.999285, 0.020415], // N105
    [0.031837, 0.999285, 0.020415], // N106
    [0.014647, 0.999648, 0.022115], // N107
    [0.014647, 0.999648, 0.022115], // N108
    [0.014647, 0.999648, 0.022115], // N109
    [-0.985141, 0.039475, 0.167149], // N110
    [-0.985141, 0.039475, 0.167149], // N111
    [-0.985141, 0.039475, 0.167149], // N112
    [0.0, 1.0, 0.0], // N113
    [0.0, 1.0, 0.0], // N114
    [0.0, 1.0, 0.0], // N115
    [0.0, 1.0, 0.0], // N116
    [0.0, 1.0, 0.0], // N117
    [0.0, 1.0, 0.0], // N118
    [0.0, 1.0, 0.0], // N119
    [0.0, 1.0, 0.0], // N120
    [0.0, 1.0, 0.0], // N121
];

/// `whale`'s points at rest, which are upstream's `P001` and
/// its immutable copy `iP001` both: the two are declared with the same
/// values and only the working copy is ever written to.
#[rustfmt::skip]
pub const WHALE_P: [[f32; 3]; 122] = [
    [0.0, 0.0, 0.0],
    [18.74, 13.19, 3.76], // P001
    [0.0, 390.42, 10292.57], // P002
    [55.8, 622.31, 8254.35], // P003
    [20.8, 247.66, 10652.13], // P004
    [487.51, 198.05, 9350.78], // P005
    [-457.61, 199.04, 9353.01], // P006
    [0.0, 259.0, 10276.27], // P007
    [-34.67, 247.64, 10663.71], // P008
    [97.46, 67.63, 593.82], // P009
    [-84.33, 67.63, 588.18], // P010
    [118.69, 8.98, -66.91], // P011
    [156.48, -31.95, 924.54], // P012
    [162.0, 110.22, 924.54], // P013
    [88.16, 221.65, 924.54], // P014
    [-65.21, 231.16, 924.54], // P015
    [-156.48, 121.97, 924.54], // P016
    [-162.0, -23.93, 924.54], // P017
    [-88.16, -139.1, 924.54], // P018
    [65.21, -148.61, 924.54], // P019
    [246.87, -98.73, 1783.04], // P020
    [253.17, 127.76, 1783.04], // P021
    [132.34, 270.77, 1783.04], // P022
    [-97.88, 285.04, 1783.04], // P023
    [-222.97, 139.8, 1783.04], // P024
    [-225.29, -86.68, 1783.04], // P025
    [-108.44, -224.15, 1783.04], // P026
    [97.88, -221.56, 1783.04], // P027
    [410.55, -200.66, 3213.87], // P028
    [432.19, 148.42, 3213.87], // P029
    [200.66, 410.55, 3213.87], // P030
    [-148.42, 432.19, 3213.87], // P031
    [-407.48, 171.88, 3213.87], // P032
    [-432.19, -148.42, 3213.87], // P033
    [-148.88, -309.74, 3213.87], // P034
    [156.38, -320.17, 3213.87], // P035
    [523.39, -303.81, 4424.57], // P036
    [574.66, 276.84, 4424.57], // P037
    [243.05, 492.5, 4424.57], // P038
    [-191.23, 520.13, 4424.57], // P039
    [-523.39, 304.01, 4424.57], // P040
    [-574.66, -231.83, 4424.57], // P041
    [-266.95, -578.17, 4424.57], // P042
    [211.14, -579.67, 4424.57], // P043
    [680.57, -370.27, 5943.46], // P044
    [834.01, 363.09, 5943.46], // P045
    [371.29, 614.13, 5943.46], // P046
    [-291.43, 621.86, 5943.46], // P047
    [-784.13, 362.6, 5943.46], // P048
    [-743.29, -325.82, 5943.46], // P049
    [-383.24, -804.77, 5943.46], // P050
    [283.47, -846.09, 5943.46], // P051
    [599.09, -332.24, 7902.59], // P052
    [735.48, 306.26, 7911.92], // P053
    [321.55, 558.53, 7902.59], // P054
    [-260.54, 559.84, 7902.59], // P055
    [-698.66, 320.83, 7902.59], // P056
    [-643.29, -299.16, 7902.59], // P057
    [-341.47, -719.3, 7902.59], // P058
    [252.57, -756.12, 7902.59], // P059
    [458.39, -265.31, 9355.44], // P060
    [353.63, 138.7, 10214.2], // P061
    [224.04, 438.98, 9364.77], // P062
    [-165.71, 441.27, 9355.44], // P063
    [-326.4, 162.04, 10209.54], // P064
    [-473.99, -219.71, 9355.44], // P065
    [-211.97, -479.87, 9355.44], // P066
    [192.86, -504.03, 9355.44], // P067
    [-112.44, 9.25, -64.42], // P068
    [1155.63, 0.0, -182.46], // P069
    [-1143.13, 0.0, -181.54], // P070
    [1424.23, 0.0, -322.09], // P071
    [-1368.01, 0.0, -310.38], // P072
    [1255.57, 2.31, 114.05], // P073
    [-1149.38, 0.0, 117.12], // P074
    [718.36, 0.0, 433.36], // P075
    [-655.9, 0.0, 433.36], // P076
    [1058.0, -2.66, 7923.51], // P077
    [-1016.51, -15.47, 7902.87], // P078
    [-1363.99, -484.5, 7593.38], // P079
    [1478.09, -861.47, 7098.12], // P080
    [1338.06, -284.68, 7024.15], // P081
    [-1545.51, -860.64, 7106.6], // P082
    [1063.19, -70.46, 7466.6], // P083
    [-1369.18, -288.11, 7015.34], // P084
    [1348.44, -482.5, 7591.41], // P085
    [-1015.45, -96.8, 7474.86], // P086
    [731.04, 148.38, 7682.58], // P087
    [-697.03, 151.82, 7668.81], // P088
    [-686.82, 157.09, 7922.29], // P089
    [724.73, 147.75, 7931.39], // P090
    [0.0, 327.1, 2346.55], // P091
    [0.0, 552.28, 2311.31], // P092
    [0.0, 721.16, 2166.41], // P093
    [0.0, 693.42, 2388.8], // P094
    [0.0, 389.44, 2859.97], // P095
    [222.02, -183.67, 10266.89], // P096
    [-128.9, -182.7, 10266.89], // P097
    [41.04, 88.31, 10659.36], // P098
    [-48.73, 88.3, 10659.36], // P099
    [0.0, 603.42, 9340.68], // P100
    [5.7, 567.0, 7862.98], // P101
    [521.61, 156.61, 9162.34], // P102
    [83.68, 566.67, 7861.26], // P103
    [-9.86, 567.62, 7858.65], // P104
    [31.96, 565.27, 7908.46], // P105
    [22.75, 568.13, 7782.83], // P106
    [58.93, 568.42, 7775.94], // P107
    [55.91, 565.59, 7905.86], // P108
    [99.21, 566.0, 7858.65], // P109
    [-498.83, 148.14, 9135.1], // P110
    [-495.46, 133.24, 9158.48], // P111
    [-490.82, 146.23, 9182.76], // P112
    [-489.55, 174.11, 9183.66], // P113
    [-492.92, 189.0, 9160.28], // P114
    [-497.56, 176.02, 9136.0], // P115
    [526.54, 169.68, 9137.7], // P116
    [523.49, 184.85, 9161.42], // P117
    [518.56, 171.78, 9186.06], // P118
    [516.68, 143.53, 9186.98], // P119
    [519.73, 128.36, 9163.26], // P120
    [524.66, 141.43, 9138.62], // P121
];

pub fn whale001(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[1]);
    f.normal(WHALE_N[68]);
    f.normal(WHALE_N[10]);
    f.poly(&[p[1], p[68], p[10]]);
    f.normal(WHALE_N[68]);
    f.normal(WHALE_N[76]);
    f.normal(WHALE_N[10]);
    f.poly(&[p[68], p[76], p[10]]);
    f.normal(WHALE_N[68]);
    f.normal(WHALE_N[70]);
    f.normal(WHALE_N[76]);
    f.poly(&[p[68], p[70], p[76]]);
    f.normal(WHALE_N[76]);
    f.normal(WHALE_N[70]);
    f.normal(WHALE_N[74]);
    f.poly(&[p[76], p[70], p[74]]);
    f.normal(WHALE_N[70]);
    f.normal(WHALE_N[72]);
    f.normal(WHALE_N[74]);
    f.poly(&[p[70], p[72], p[74]]);
    f.normal(WHALE_N[72]);
    f.normal(WHALE_N[70]);
    f.normal(WHALE_N[74]);
    f.poly(&[p[72], p[70], p[74]]);
    f.normal(WHALE_N[74]);
    f.normal(WHALE_N[70]);
    f.normal(WHALE_N[76]);
    f.poly(&[p[74], p[70], p[76]]);
    f.normal(WHALE_N[70]);
    f.normal(WHALE_N[68]);
    f.normal(WHALE_N[76]);
    f.poly(&[p[70], p[68], p[76]]);
    f.normal(WHALE_N[76]);
    f.normal(WHALE_N[68]);
    f.normal(WHALE_N[10]);
    f.poly(&[p[76], p[68], p[10]]);
    f.normal(WHALE_N[68]);
    f.normal(WHALE_N[1]);
    f.normal(WHALE_N[10]);
    f.poly(&[p[68], p[1], p[10]]);
}

pub fn whale002(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[11]);
    f.normal(WHALE_N[1]);
    f.normal(WHALE_N[9]);
    f.poly(&[p[11], p[1], p[9]]);
    f.normal(WHALE_N[75]);
    f.normal(WHALE_N[11]);
    f.normal(WHALE_N[9]);
    f.poly(&[p[75], p[11], p[9]]);
    f.normal(WHALE_N[69]);
    f.normal(WHALE_N[11]);
    f.normal(WHALE_N[75]);
    f.poly(&[p[69], p[11], p[75]]);
    f.normal(WHALE_N[69]);
    f.normal(WHALE_N[75]);
    f.normal(WHALE_N[73]);
    f.poly(&[p[69], p[75], p[73]]);
    f.normal(WHALE_N[71]);
    f.normal(WHALE_N[69]);
    f.normal(WHALE_N[73]);
    f.poly(&[p[71], p[69], p[73]]);
    f.normal(WHALE_N[1]);
    f.normal(WHALE_N[11]);
    f.normal(WHALE_N[9]);
    f.poly(&[p[1], p[11], p[9]]);
    f.normal(WHALE_N[9]);
    f.normal(WHALE_N[11]);
    f.normal(WHALE_N[75]);
    f.poly(&[p[9], p[11], p[75]]);
    f.normal(WHALE_N[11]);
    f.normal(WHALE_N[69]);
    f.normal(WHALE_N[75]);
    f.poly(&[p[11], p[69], p[75]]);
    f.normal(WHALE_N[69]);
    f.normal(WHALE_N[73]);
    f.normal(WHALE_N[75]);
    f.poly(&[p[69], p[73], p[75]]);
    f.normal(WHALE_N[69]);
    f.normal(WHALE_N[71]);
    f.normal(WHALE_N[73]);
    f.poly(&[p[69], p[71], p[73]]);
}

pub fn whale003(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[18]);
    f.normal(WHALE_N[1]);
    f.normal(WHALE_N[19]);
    f.poly(&[p[18], p[1], p[19]]);
    f.normal(WHALE_N[19]);
    f.normal(WHALE_N[1]);
    f.normal(WHALE_N[12]);
    f.poly(&[p[19], p[1], p[12]]);
    f.normal(WHALE_N[17]);
    f.normal(WHALE_N[1]);
    f.normal(WHALE_N[18]);
    f.poly(&[p[17], p[1], p[18]]);
    f.normal(WHALE_N[1]);
    f.normal(WHALE_N[17]);
    f.normal(WHALE_N[16]);
    f.poly(&[p[1], p[17], p[16]]);
    f.normal(WHALE_N[1]);
    f.normal(WHALE_N[13]);
    f.normal(WHALE_N[12]);
    f.poly(&[p[1], p[13], p[12]]);
    f.normal(WHALE_N[1]);
    f.normal(WHALE_N[16]);
    f.normal(WHALE_N[15]);
    f.poly(&[p[1], p[16], p[15]]);
    f.normal(WHALE_N[1]);
    f.normal(WHALE_N[14]);
    f.normal(WHALE_N[13]);
    f.poly(&[p[1], p[14], p[13]]);
    f.normal(WHALE_N[1]);
    f.normal(WHALE_N[15]);
    f.normal(WHALE_N[14]);
    f.poly(&[p[1], p[15], p[14]]);
}

pub fn whale004(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[14]);
    f.normal(WHALE_N[15]);
    f.normal(WHALE_N[23]);
    f.normal(WHALE_N[22]);
    f.poly(&[p[14], p[15], p[23], p[22]]);
    f.normal(WHALE_N[15]);
    f.normal(WHALE_N[16]);
    f.normal(WHALE_N[24]);
    f.normal(WHALE_N[23]);
    f.poly(&[p[15], p[16], p[24], p[23]]);
    f.normal(WHALE_N[16]);
    f.normal(WHALE_N[17]);
    f.normal(WHALE_N[25]);
    f.normal(WHALE_N[24]);
    f.poly(&[p[16], p[17], p[25], p[24]]);
    f.normal(WHALE_N[17]);
    f.normal(WHALE_N[18]);
    f.normal(WHALE_N[26]);
    f.normal(WHALE_N[25]);
    f.poly(&[p[17], p[18], p[26], p[25]]);
    f.normal(WHALE_N[13]);
    f.normal(WHALE_N[14]);
    f.normal(WHALE_N[22]);
    f.normal(WHALE_N[21]);
    f.poly(&[p[13], p[14], p[22], p[21]]);
    f.normal(WHALE_N[12]);
    f.normal(WHALE_N[13]);
    f.normal(WHALE_N[21]);
    f.normal(WHALE_N[20]);
    f.poly(&[p[12], p[13], p[21], p[20]]);
    f.normal(WHALE_N[18]);
    f.normal(WHALE_N[19]);
    f.normal(WHALE_N[27]);
    f.normal(WHALE_N[26]);
    f.poly(&[p[18], p[19], p[27], p[26]]);
    f.normal(WHALE_N[19]);
    f.normal(WHALE_N[12]);
    f.normal(WHALE_N[20]);
    f.normal(WHALE_N[27]);
    f.poly(&[p[19], p[12], p[20], p[27]]);
}

pub fn whale005(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[22]);
    f.normal(WHALE_N[23]);
    f.normal(WHALE_N[31]);
    f.normal(WHALE_N[30]);
    f.poly(&[p[22], p[23], p[31], p[30]]);
    f.normal(WHALE_N[21]);
    f.normal(WHALE_N[22]);
    f.normal(WHALE_N[30]);
    f.poly(&[p[21], p[22], p[30]]);
    f.normal(WHALE_N[21]);
    f.normal(WHALE_N[30]);
    f.normal(WHALE_N[29]);
    f.poly(&[p[21], p[30], p[29]]);
    f.normal(WHALE_N[23]);
    f.normal(WHALE_N[24]);
    f.normal(WHALE_N[31]);
    f.poly(&[p[23], p[24], p[31]]);
    f.normal(WHALE_N[24]);
    f.normal(WHALE_N[32]);
    f.normal(WHALE_N[31]);
    f.poly(&[p[24], p[32], p[31]]);
    f.normal(WHALE_N[24]);
    f.normal(WHALE_N[25]);
    f.normal(WHALE_N[32]);
    f.poly(&[p[24], p[25], p[32]]);
    f.normal(WHALE_N[25]);
    f.normal(WHALE_N[33]);
    f.normal(WHALE_N[32]);
    f.poly(&[p[25], p[33], p[32]]);
    f.normal(WHALE_N[20]);
    f.normal(WHALE_N[21]);
    f.normal(WHALE_N[29]);
    f.poly(&[p[20], p[21], p[29]]);
    f.normal(WHALE_N[20]);
    f.normal(WHALE_N[29]);
    f.normal(WHALE_N[28]);
    f.poly(&[p[20], p[29], p[28]]);
    f.normal(WHALE_N[27]);
    f.normal(WHALE_N[20]);
    f.normal(WHALE_N[28]);
    f.poly(&[p[27], p[20], p[28]]);
    f.normal(WHALE_N[27]);
    f.normal(WHALE_N[28]);
    f.normal(WHALE_N[35]);
    f.poly(&[p[27], p[28], p[35]]);
    f.normal(WHALE_N[25]);
    f.normal(WHALE_N[26]);
    f.normal(WHALE_N[33]);
    f.poly(&[p[25], p[26], p[33]]);
    f.normal(WHALE_N[33]);
    f.normal(WHALE_N[26]);
    f.normal(WHALE_N[34]);
    f.poly(&[p[33], p[26], p[34]]);
    f.normal(WHALE_N[26]);
    f.normal(WHALE_N[27]);
    f.normal(WHALE_N[35]);
    f.normal(WHALE_N[34]);
    f.poly(&[p[26], p[27], p[35], p[34]]);
}

pub fn whale006(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[92]);
    f.normal(WHALE_N[93]);
    f.normal(WHALE_N[94]);
    f.poly(&[p[92], p[93], p[94]]);
    f.normal(WHALE_N[93]);
    f.normal(WHALE_N[92]);
    f.normal(WHALE_N[94]);
    f.poly(&[p[93], p[92], p[94]]);
    f.normal(WHALE_N[92]);
    f.normal(WHALE_N[91]);
    f.normal(WHALE_N[95]);
    f.normal(WHALE_N[94]);
    f.poly(&[p[92], p[91], p[95], p[94]]);
    f.normal(WHALE_N[91]);
    f.normal(WHALE_N[92]);
    f.normal(WHALE_N[94]);
    f.normal(WHALE_N[95]);
    f.poly(&[p[91], p[92], p[94], p[95]]);
}

pub fn whale007(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[30]);
    f.normal(WHALE_N[31]);
    f.normal(WHALE_N[39]);
    f.normal(WHALE_N[38]);
    f.poly(&[p[30], p[31], p[39], p[38]]);
    f.normal(WHALE_N[29]);
    f.normal(WHALE_N[30]);
    f.normal(WHALE_N[38]);
    f.poly(&[p[29], p[30], p[38]]);
    f.normal(WHALE_N[29]);
    f.normal(WHALE_N[38]);
    f.normal(WHALE_N[37]);
    f.poly(&[p[29], p[38], p[37]]);
    f.normal(WHALE_N[28]);
    f.normal(WHALE_N[29]);
    f.normal(WHALE_N[37]);
    f.poly(&[p[28], p[29], p[37]]);
    f.normal(WHALE_N[28]);
    f.normal(WHALE_N[37]);
    f.normal(WHALE_N[36]);
    f.poly(&[p[28], p[37], p[36]]);
    f.normal(WHALE_N[35]);
    f.normal(WHALE_N[28]);
    f.normal(WHALE_N[36]);
    f.poly(&[p[35], p[28], p[36]]);
    f.normal(WHALE_N[35]);
    f.normal(WHALE_N[36]);
    f.normal(WHALE_N[43]);
    f.poly(&[p[35], p[36], p[43]]);
    f.normal(WHALE_N[34]);
    f.normal(WHALE_N[35]);
    f.normal(WHALE_N[43]);
    f.normal(WHALE_N[42]);
    f.poly(&[p[34], p[35], p[43], p[42]]);
    f.normal(WHALE_N[33]);
    f.normal(WHALE_N[34]);
    f.normal(WHALE_N[42]);
    f.poly(&[p[33], p[34], p[42]]);
    f.normal(WHALE_N[33]);
    f.normal(WHALE_N[42]);
    f.normal(WHALE_N[41]);
    f.poly(&[p[33], p[42], p[41]]);
    f.normal(WHALE_N[31]);
    f.normal(WHALE_N[32]);
    f.normal(WHALE_N[39]);
    f.poly(&[p[31], p[32], p[39]]);
    f.normal(WHALE_N[39]);
    f.normal(WHALE_N[32]);
    f.normal(WHALE_N[40]);
    f.poly(&[p[39], p[32], p[40]]);
    f.normal(WHALE_N[32]);
    f.normal(WHALE_N[33]);
    f.normal(WHALE_N[40]);
    f.poly(&[p[32], p[33], p[40]]);
    f.normal(WHALE_N[40]);
    f.normal(WHALE_N[33]);
    f.normal(WHALE_N[41]);
    f.poly(&[p[40], p[33], p[41]]);
}

pub fn whale008(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[42]);
    f.normal(WHALE_N[43]);
    f.normal(WHALE_N[51]);
    f.normal(WHALE_N[50]);
    f.poly(&[p[42], p[43], p[51], p[50]]);
    f.normal(WHALE_N[43]);
    f.normal(WHALE_N[36]);
    f.normal(WHALE_N[51]);
    f.poly(&[p[43], p[36], p[51]]);
    f.normal(WHALE_N[51]);
    f.normal(WHALE_N[36]);
    f.normal(WHALE_N[44]);
    f.poly(&[p[51], p[36], p[44]]);
    f.normal(WHALE_N[41]);
    f.normal(WHALE_N[42]);
    f.normal(WHALE_N[50]);
    f.poly(&[p[41], p[42], p[50]]);
    f.normal(WHALE_N[41]);
    f.normal(WHALE_N[50]);
    f.normal(WHALE_N[49]);
    f.poly(&[p[41], p[50], p[49]]);
    f.normal(WHALE_N[36]);
    f.normal(WHALE_N[37]);
    f.normal(WHALE_N[44]);
    f.poly(&[p[36], p[37], p[44]]);
    f.normal(WHALE_N[44]);
    f.normal(WHALE_N[37]);
    f.normal(WHALE_N[45]);
    f.poly(&[p[44], p[37], p[45]]);
    f.normal(WHALE_N[40]);
    f.normal(WHALE_N[41]);
    f.normal(WHALE_N[49]);
    f.poly(&[p[40], p[41], p[49]]);
    f.normal(WHALE_N[40]);
    f.normal(WHALE_N[49]);
    f.normal(WHALE_N[48]);
    f.poly(&[p[40], p[49], p[48]]);
    f.normal(WHALE_N[39]);
    f.normal(WHALE_N[40]);
    f.normal(WHALE_N[48]);
    f.poly(&[p[39], p[40], p[48]]);
    f.normal(WHALE_N[39]);
    f.normal(WHALE_N[48]);
    f.normal(WHALE_N[47]);
    f.poly(&[p[39], p[48], p[47]]);
    f.normal(WHALE_N[37]);
    f.normal(WHALE_N[38]);
    f.normal(WHALE_N[45]);
    f.poly(&[p[37], p[38], p[45]]);
    f.normal(WHALE_N[38]);
    f.normal(WHALE_N[46]);
    f.normal(WHALE_N[45]);
    f.poly(&[p[38], p[46], p[45]]);
    f.normal(WHALE_N[38]);
    f.normal(WHALE_N[39]);
    f.normal(WHALE_N[47]);
    f.normal(WHALE_N[46]);
    f.poly(&[p[38], p[39], p[47], p[46]]);
}

pub fn whale009(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[50]);
    f.normal(WHALE_N[51]);
    f.normal(WHALE_N[59]);
    f.normal(WHALE_N[58]);
    f.poly(&[p[50], p[51], p[59], p[58]]);
    f.normal(WHALE_N[51]);
    f.normal(WHALE_N[44]);
    f.normal(WHALE_N[59]);
    f.poly(&[p[51], p[44], p[59]]);
    f.normal(WHALE_N[59]);
    f.normal(WHALE_N[44]);
    f.normal(WHALE_N[52]);
    f.poly(&[p[59], p[44], p[52]]);
    f.normal(WHALE_N[44]);
    f.normal(WHALE_N[45]);
    f.normal(WHALE_N[53]);
    f.poly(&[p[44], p[45], p[53]]);
    f.normal(WHALE_N[44]);
    f.normal(WHALE_N[53]);
    f.normal(WHALE_N[52]);
    f.poly(&[p[44], p[53], p[52]]);
    f.normal(WHALE_N[49]);
    f.normal(WHALE_N[50]);
    f.normal(WHALE_N[58]);
    f.poly(&[p[49], p[50], p[58]]);
    f.normal(WHALE_N[49]);
    f.normal(WHALE_N[58]);
    f.normal(WHALE_N[57]);
    f.poly(&[p[49], p[58], p[57]]);
    f.normal(WHALE_N[48]);
    f.normal(WHALE_N[49]);
    f.normal(WHALE_N[57]);
    f.poly(&[p[48], p[49], p[57]]);
    f.normal(WHALE_N[48]);
    f.normal(WHALE_N[57]);
    f.normal(WHALE_N[56]);
    f.poly(&[p[48], p[57], p[56]]);
    f.normal(WHALE_N[47]);
    f.normal(WHALE_N[48]);
    f.normal(WHALE_N[56]);
    f.poly(&[p[47], p[48], p[56]]);
    f.normal(WHALE_N[47]);
    f.normal(WHALE_N[56]);
    f.normal(WHALE_N[55]);
    f.poly(&[p[47], p[56], p[55]]);
    f.normal(WHALE_N[45]);
    f.normal(WHALE_N[46]);
    f.normal(WHALE_N[53]);
    f.poly(&[p[45], p[46], p[53]]);
    f.normal(WHALE_N[46]);
    f.normal(WHALE_N[54]);
    f.normal(WHALE_N[53]);
    f.poly(&[p[46], p[54], p[53]]);
    f.normal(WHALE_N[46]);
    f.normal(WHALE_N[47]);
    f.normal(WHALE_N[55]);
    f.normal(WHALE_N[54]);
    f.poly(&[p[46], p[47], p[55], p[54]]);
}

pub fn whale010(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[80]);
    f.normal(WHALE_N[81]);
    f.normal(WHALE_N[85]);
    f.poly(&[p[80], p[81], p[85]]);
    f.normal(WHALE_N[81]);
    f.normal(WHALE_N[83]);
    f.normal(WHALE_N[85]);
    f.poly(&[p[81], p[83], p[85]]);
    f.normal(WHALE_N[85]);
    f.normal(WHALE_N[83]);
    f.normal(WHALE_N[77]);
    f.poly(&[p[85], p[83], p[77]]);
    f.normal(WHALE_N[83]);
    f.normal(WHALE_N[87]);
    f.normal(WHALE_N[77]);
    f.poly(&[p[83], p[87], p[77]]);
    f.normal(WHALE_N[77]);
    f.normal(WHALE_N[87]);
    f.normal(WHALE_N[90]);
    f.poly(&[p[77], p[87], p[90]]);
    f.normal(WHALE_N[81]);
    f.normal(WHALE_N[80]);
    f.normal(WHALE_N[85]);
    f.poly(&[p[81], p[80], p[85]]);
    f.normal(WHALE_N[83]);
    f.normal(WHALE_N[81]);
    f.normal(WHALE_N[85]);
    f.poly(&[p[83], p[81], p[85]]);
    f.normal(WHALE_N[83]);
    f.normal(WHALE_N[85]);
    f.normal(WHALE_N[77]);
    f.poly(&[p[83], p[85], p[77]]);
    f.normal(WHALE_N[87]);
    f.normal(WHALE_N[83]);
    f.normal(WHALE_N[77]);
    f.poly(&[p[87], p[83], p[77]]);
    f.normal(WHALE_N[87]);
    f.normal(WHALE_N[77]);
    f.normal(WHALE_N[90]);
    f.poly(&[p[87], p[77], p[90]]);
}

pub fn whale011(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[82]);
    f.normal(WHALE_N[84]);
    f.normal(WHALE_N[79]);
    f.poly(&[p[82], p[84], p[79]]);
    f.normal(WHALE_N[84]);
    f.normal(WHALE_N[86]);
    f.normal(WHALE_N[79]);
    f.poly(&[p[84], p[86], p[79]]);
    f.normal(WHALE_N[79]);
    f.normal(WHALE_N[86]);
    f.normal(WHALE_N[78]);
    f.poly(&[p[79], p[86], p[78]]);
    f.normal(WHALE_N[86]);
    f.normal(WHALE_N[88]);
    f.normal(WHALE_N[78]);
    f.poly(&[p[86], p[88], p[78]]);
    f.normal(WHALE_N[78]);
    f.normal(WHALE_N[88]);
    f.normal(WHALE_N[89]);
    f.poly(&[p[78], p[88], p[89]]);
    f.normal(WHALE_N[88]);
    f.normal(WHALE_N[86]);
    f.normal(WHALE_N[89]);
    f.poly(&[p[88], p[86], p[89]]);
    f.normal(WHALE_N[89]);
    f.normal(WHALE_N[86]);
    f.normal(WHALE_N[78]);
    f.poly(&[p[89], p[86], p[78]]);
    f.normal(WHALE_N[86]);
    f.normal(WHALE_N[84]);
    f.normal(WHALE_N[78]);
    f.poly(&[p[86], p[84], p[78]]);
    f.normal(WHALE_N[78]);
    f.normal(WHALE_N[84]);
    f.normal(WHALE_N[79]);
    f.poly(&[p[78], p[84], p[79]]);
    f.normal(WHALE_N[84]);
    f.normal(WHALE_N[82]);
    f.normal(WHALE_N[79]);
    f.poly(&[p[84], p[82], p[79]]);
}

pub fn whale012(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[58]);
    f.normal(WHALE_N[59]);
    f.normal(WHALE_N[67]);
    f.normal(WHALE_N[66]);
    f.poly(&[p[58], p[59], p[67], p[66]]);
    f.normal(WHALE_N[59]);
    f.normal(WHALE_N[52]);
    f.normal(WHALE_N[60]);
    f.poly(&[p[59], p[52], p[60]]);
    f.normal(WHALE_N[59]);
    f.normal(WHALE_N[60]);
    f.normal(WHALE_N[67]);
    f.poly(&[p[59], p[60], p[67]]);
    f.normal(WHALE_N[58]);
    f.normal(WHALE_N[66]);
    f.normal(WHALE_N[65]);
    f.poly(&[p[58], p[66], p[65]]);
    f.normal(WHALE_N[58]);
    f.normal(WHALE_N[65]);
    f.normal(WHALE_N[57]);
    f.poly(&[p[58], p[65], p[57]]);
    f.normal(WHALE_N[56]);
    f.normal(WHALE_N[57]);
    f.normal(WHALE_N[65]);
    f.poly(&[p[56], p[57], p[65]]);
    f.normal(WHALE_N[56]);
    f.normal(WHALE_N[65]);
    f.normal(WHALE_N[6]);
    f.poly(&[p[56], p[65], p[6]]);
    f.normal(WHALE_N[56]);
    f.normal(WHALE_N[6]);
    f.normal(WHALE_N[63]);
    f.poly(&[p[56], p[6], p[63]]);
    f.normal(WHALE_N[56]);
    f.normal(WHALE_N[63]);
    f.normal(WHALE_N[55]);
    f.poly(&[p[56], p[63], p[55]]);
    f.normal(WHALE_N[54]);
    f.normal(WHALE_N[62]);
    f.normal(WHALE_N[5]);
    f.poly(&[p[54], p[62], p[5]]);
    f.normal(WHALE_N[54]);
    f.normal(WHALE_N[5]);
    f.normal(WHALE_N[53]);
    f.poly(&[p[54], p[5], p[53]]);
    f.normal(WHALE_N[53]);
    f.normal(WHALE_N[5]);
    f.normal(WHALE_N[60]);
    f.poly(&[p[53], p[5], p[60]]);
    f.normal(WHALE_N[53]);
    f.normal(WHALE_N[60]);
    f.normal(WHALE_N[52]);
    f.poly(&[p[53], p[60], p[52]]);
}

pub fn whale013(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[66]);
    f.normal(WHALE_N[67]);
    f.normal(WHALE_N[96]);
    f.normal(WHALE_N[97]);
    f.poly(&[p[66], p[67], p[96], p[97]]);
    f.normal(WHALE_N[97]);
    f.normal(WHALE_N[96]);
    f.normal(WHALE_N[98]);
    f.normal(WHALE_N[99]);
    f.poly(&[p[97], p[96], p[98], p[99]]);
    f.normal(WHALE_N[65]);
    f.normal(WHALE_N[66]);
    f.normal(WHALE_N[97]);
    f.poly(&[p[65], p[66], p[97]]);
    f.normal(WHALE_N[67]);
    f.normal(WHALE_N[60]);
    f.normal(WHALE_N[96]);
    f.poly(&[p[67], p[60], p[96]]);
    f.normal(WHALE_N[60]);
    f.normal(WHALE_N[5]);
    f.normal(WHALE_N[96]);
    f.poly(&[p[60], p[5], p[96]]);
    f.normal(WHALE_N[96]);
    f.normal(WHALE_N[5]);
    f.normal(WHALE_N[98]);
    f.poly(&[p[96], p[5], p[98]]);
    f.normal(WHALE_N[6]);
    f.normal(WHALE_N[65]);
    f.normal(WHALE_N[97]);
    f.poly(&[p[6], p[65], p[97]]);
    f.normal(WHALE_N[6]);
    f.normal(WHALE_N[97]);
    f.normal(WHALE_N[99]);
    f.poly(&[p[6], p[97], p[99]]);
    f.poly(&[p[5], p[6], p[99], p[98]]);
}

pub fn whale014(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[62]);
    f.normal(WHALE_N[4]);
    f.normal(WHALE_N[5]);
    f.poly(&[p[62], p[4], p[5]]);
    f.poly(&[p[6], p[5], p[4], p[8]]);
    f.normal(WHALE_N[63]);
    f.normal(WHALE_N[6]);
    f.normal(WHALE_N[2]);
    f.poly(&[p[63], p[6], p[2]]);
    f.normal(WHALE_N[2]);
    f.normal(WHALE_N[6]);
    f.normal(WHALE_N[8]);
    f.poly(&[p[2], p[6], p[8]]);
    f.normal(WHALE_N[2]);
    f.normal(WHALE_N[8]);
    f.normal(WHALE_N[4]);
    f.poly(&[p[2], p[8], p[4]]);
    f.normal(WHALE_N[62]);
    f.normal(WHALE_N[2]);
    f.normal(WHALE_N[4]);
    f.poly(&[p[62], p[2], p[4]]);
}

pub fn whale015(f: &mut Fish, p: &Pts) {
    f.normal(WHALE_N[55]);
    f.normal(WHALE_N[3]);
    f.normal(WHALE_N[54]);
    f.poly(&[p[55], p[3], p[54]]);
    f.normal(WHALE_N[3]);
    f.normal(WHALE_N[55]);
    f.normal(WHALE_N[63]);
    f.poly(&[p[3], p[55], p[63]]);
    f.normal(WHALE_N[3]);
    f.normal(WHALE_N[63]);
    f.normal(WHALE_N[100]);
    f.poly(&[p[3], p[63], p[100]]);
    f.normal(WHALE_N[3]);
    f.normal(WHALE_N[100]);
    f.normal(WHALE_N[54]);
    f.poly(&[p[3], p[100], p[54]]);
    f.normal(WHALE_N[54]);
    f.normal(WHALE_N[100]);
    f.normal(WHALE_N[62]);
    f.poly(&[p[54], p[100], p[62]]);
    f.normal(WHALE_N[100]);
    f.normal(WHALE_N[63]);
    f.normal(WHALE_N[2]);
    f.poly(&[p[100], p[63], p[2]]);
    f.normal(WHALE_N[100]);
    f.normal(WHALE_N[2]);
    f.normal(WHALE_N[62]);
    f.poly(&[p[100], p[2], p[62]]);
}

pub fn whale016(f: &mut Fish, p: &Pts) {
    f.poly(&[p[104], p[105], p[106]]);
    f.poly(&[p[107], p[108], p[109]]);
    f.poly(&[p[110], p[111], p[112], p[113], p[114], p[115]]);
    f.poly(&[p[116], p[117], p[118], p[119], p[120], p[121]]);
}
