#ifdef GL_ES
  precision highp float;
  precision highp int;
#endif

#if __VERSION__ <= 120
  attribute vec2 a_Position;
#else
  in vec2 a_Position;
#endif

void main() {
  gl_Position = vec4 (a_Position.xy, 0.0, 1.0);
}
