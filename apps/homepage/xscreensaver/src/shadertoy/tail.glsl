
void main() {
  vec4 col = vec4 (0.0, 0.0, 0.0, 1.0);
  mainImage (col, gl_FragCoord.xy);
#if __VERSION__ <= 120
  gl_FragColor = col;
#else
  frag_color = col;
#endif
}
