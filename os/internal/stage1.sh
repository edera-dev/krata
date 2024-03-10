#!/bin/sh
set -e

apk add --update-cache alpine-base \
  linux-lts linux-firmware-none \
  mkinitfs dosfstools e2fsprogs \
  tzdata chrony

apk add --allow-untrusted /mnt/target/os/krata.apk

for SERVICE in kratad kratanet
do
  rc-update add "${SERVICE}" default
done

apk add xen xen-hypervisor

for SERVICE in xenconsoled xenstored
do
  rc-update add "${SERVICE}" default
done

for MODULE in xen-netblock xen-blkback tun tap
do
  echo "${MODULE}" >> /etc/modules
done

cat > /etc/network/interfaces <<-EOF
  auto eth0
	iface eth0 inet dhcp
EOF

for SERVICE in networking chronyd
do
  rc-update add "${SERVICE}" default
done

for SERVICE in devfs dmesg mdev hwdrivers cgroups
do
  rc-update add "${SERVICE}" sysinit
done

for SERVICE in modules hwclock swap hostname sysctl bootmisc syslog seedrng
do
  rc-update add "${SERVICE}" boot
done

for SERVICE in killprocs savecache mount-ro
do
  rc-update add "${SERVICE}" shutdown
done

echo 'root:krata' | chpasswd
echo 'krata' > /etc/hostname

{
  echo '# krata resolver configuration'
  echo 'nameserver 1.1.1.1'
  echo 'nameserver 1.0.0.1'
  echo 'nameserver 2606:4700:4700::1111'
  echo 'nameserver 2606:4700:4700::1001'
} > /etc/resolv.conf

{
  echo 'Welcome to krataOS!'
  echo 'You may now login to the console to manage krata.'
} > /etc/issue

echo > /etc/motd

ln -s /usr/share/zoneinfo/UTC /etc/localtime

rm -rf /var/cache/apk/*
rm -rf /.dockerenv

cd /
rm -f /mnt/target/os/rootfs.tar
tar cf /mnt/target/os/rootfs.tar --numeric-owner \
  --exclude 'mnt/**' --exclude 'proc/**' \
  --exclude 'sys/**' --exclude 'dev/**' .
