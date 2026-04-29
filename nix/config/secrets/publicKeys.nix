rec {
  noverby-ssh-ed25519 = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOachAYzBH8Qaorvbck99Fw+v6md3BeVtfL5PJ/byv4C";
  noverby-ssh-rsa = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDVVn8JxJlkjZhyKTgkfv6fOh3PfDxaN28kstlNHqondU8pCpANVoIX8tXWeNhF5kvKrCNMW2/NTZVH8tY4a818MfHf9rHHAtFea3ZkQ7ji3hxyn/OzyNd54p4rRPp0MkrnQeg9sKg8RPf+8k5tK/B5nXdsGiTJ1C44yQAdDFstdTq+Sykd8ZZnDQf4fWRShZpbINUo0k+eWcgRnFeaSZS3yeXq9cMLcP/M8RG8WkDf50fXDGou8Qhpgib6GoYa7wtxJ4OBCtpfpBHmEjt9WNPPHRHLKeQeSRWgL2fQbpZ6gatsQLTNuu0Lux1uxXZksQKtnMUoOlMfjMlE8uVIvRW7 niclas@overby.me";
  nitrokey3-fido2-hmac = "age1efazxe5tgepdv5czxzj5x844dj265dar4sej42qc8mjp2czulazslqutuw";
  phone-ssh-ed25519 = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHhRXDKeALBSWplejvGMt4g4lc3e0W7JEM/KYPj9dM1J phone-host-key";
  home-ssh-ed25519 = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMvZkUPUbFqMZk2w/V4ZQJd0xHsD8pZEfq47Do6kiTyF";
  gravitas-ssh-ed25519 = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDBfdXLATD3DU/MGdHPVnWxy4jbv37ePZi5YJuZiywtz root@gravitas";
  all = [noverby-ssh-ed25519 noverby-ssh-rsa nitrokey3-fido2-hmac phone-ssh-ed25519 home-ssh-ed25519 gravitas-ssh-ed25519];
}
