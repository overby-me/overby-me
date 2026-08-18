{
  name = "03-JOBS";
  patchScript = ''
    # Fix upstream typo: propagatesstopto → propagatestopto
    sed -i 's/propagatesstopto-indirect/propagatestopto-indirect/g' TEST-03-JOBS.sh
  '';
}
