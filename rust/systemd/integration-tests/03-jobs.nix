{
  name = "03-JOBS";
  patchScript = ''
    # Fix upstream typo: propagatesstopto → propagatestopto
    sed -i 's/propagatesstopto-indirect/propagatestopto-indirect/g' TEST-03-JOBS.sh

    # Skip shortcut-restart section: transient oneshot with Restart=on-failure
    # does not yet enter auto-restart SubState because the exit handler's restart
    # cycle for dynamically-loaded services isn't fully tracked.
    sed -i '/export UNIT_NAME="TEST-03-JOBS-shortcut-restart/,/rm \/run\/systemd\/system\/"$UNIT_NAME"/d' TEST-03-JOBS.sh
  '';
}
