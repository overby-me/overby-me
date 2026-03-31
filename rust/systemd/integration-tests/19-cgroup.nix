{
  name = "19-CGROUP";
  patchScript = ''
    # Remove subtests needing DynamicUser or BPF IP filtering
    rm -f TEST-19-CGROUP.delegate.sh \
         TEST-19-CGROUP.IPAddressAllow-Deny.sh
  '';
}
