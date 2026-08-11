#ifdef GL_ES
  precision highp float;
  precision highp int;
#endif

#if __VERSION__ >= 130
  out vec4 frag_color;
#endif

uniform vec3  iResolution;  // viewport resolution (in pixels)
uniform float iTime;        // shader playback time (in secs)
uniform float iTimeDelta;   // render time (in secs)
uniform float iFrameRate;   // shader FPS
uniform int   iFrame;       // shader frame number
uniform vec4  iDate;        // (Y, M, D, secs since midnight)
uniform vec4  iMouse;       // mouse pos (see below)

uniform vec3  iChannelResolution[4];    // Texture sizes

// These are for audio:
uniform float iChannelTime[4];
uniform float iSampleRate;

// On the web version, these might be of type sampler3D or
// something else, depending on what input was selected in
// then menu.  Here, we only support 2D textures because we
// only allow input from the output of the previous pass.
//
uniform sampler2D iChannel0;
uniform sampler2D iChannel1;
uniform sampler2D iChannel2;
uniform sampler2D iChannel3;


// These library functions existed in GLSL 1.2 but were
// removed in GLSL 1.3; avoid name conflicts if programs
// happen to use these names for their own functions.
//
# define noise  xshadertoy_noise
# define noise2 xshadertoy_noise2


#if __VERSION__ <= 120

// The ivec and uvec types were introduced in GLSL 1.3.
#define ivec2 vec2
#define ivec3 vec3
#define ivec4 vec4
#define uvec2 vec2
#define uvec3 vec3
#define uvec4 vec4
#define uint  int

// The texture2D() function was renamed texture() in GLSL 1.3.
// (Deprecated in GLSL 1.3, removed in GLSL 1.5.)
//
vec4 texture (sampler2D sampler, vec2 coord) {
  return texture2D (sampler, coord);
}
vec4 texture (sampler2D sampler, vec3 coord) {
  return texture2D (sampler, coord.xy);
}
vec4 texture (sampler2D sampler, vec2 coord, float bias) {
  return texture2D (sampler, coord, bias);
}
vec4 texture (sampler2D sampler, vec3 coord, float bias) {
  return texture2D (sampler, coord.xy, bias);
}

vec4 texelFetch(sampler2D sampler, ivec2 coord, int lod) {
  return texture (sampler, (coord + 0.5) / iResolution.xy);
}
vec4 texelFetch(sampler2D sampler, ivec3 coord, int lod) {
  return texture (sampler, (coord + 0.5) / iResolution);
}

// Hyperbolic functions were added in GLSL 1.3.
//
float sinh(in float i) {
  // return (exp(i) - exp(-i)) / 2;
  float j = exp(i);
  return (j - 1 / j) / 2;
}
float cosh(in float i) {
  // return (exp(i) + exp(-i)) / 2;
  float j = exp(i);
  return (j + 1 / j) / 2;
}
float tanh(in float i) {
  // return sinh(i) / cosh(i);
  return (2 / (1 + exp(-2 * i))) - 1;
}
vec2 sinh(in vec2 v) { return vec2(sinh(v.x), sinh(v.y)); }
vec2 cosh(in vec2 v) { return vec2(cosh(v.x), cosh(v.y)); }
vec2 tanh(in vec2 v) { return vec2(tanh(v.x), tanh(v.y)); }

vec3 sinh(in vec3 v) {
  return vec3(sinh(v.x), sinh(v.y), sinh(v.z));
}
vec3 cosh(vec3 v) {
  return vec3(cosh(v.x), cosh(v.y), cosh(v.z));
}
vec3 tanh(vec3 v) {
  return vec3(tanh(v.x), tanh(v.y), tanh(v.z));
}

vec4 sinh(in vec4 v) {
  return vec4(sinh(v.x), sinh(v.y), sinh(v.z), sinh(v.w));
}
vec4 cosh(vec4 v) {
  return vec4(cosh(v.x), cosh(v.y), cosh(v.z), cosh(v.w));
}
vec4 tanh(vec4 v) {
  return vec4(tanh(v.x), tanh(v.y), tanh(v.z), tanh(v.w));
}

// The modern concept of rounding was added in GLSL 1.3.
//
int round(float f) { return int(f + 0.5); }
ivec2 round(vec2 v) { return ivec2(round(v.x), round(v.y)); }
ivec3 round(vec3 v) {
  return ivec3(round(v.x), round(v.y), round(v.z));
}
ivec4 round(vec4 v) {
  return ivec4(round(v.x), round(v.y), round(v.z), round(v.w));
}

// I think the type propagation rules changed?
// This makes 'int i = max(...)' work.
int max(int a, int b) { return a > b ? a : b; }
// I hate that GLSL's method-selection signatures include
// arg types but not return types, yet require them to match,
// so these conflict:
// int max(int a, float b) { return a > b ? a : int(b); }
// int max(float a, int b) { return a > b ? int(a) : b; }

// Another common compatibility problem that I have noticed
// is that for things to work on version 120, variables need
// to be initialized, not assumed to be zero.

#endif   // __VERSION__ > 120

#line 0
