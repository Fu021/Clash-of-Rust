#!/bin/sh
set -eu

uid=$1
gid=$2
shift 2
export DEBIAN_FRONTEND=noninteractive

apt-get -o Acquire::Retries=2 -o APT::Update::Error-Mode=any update
apt-get -o Acquire::Retries=2 install -y --no-install-recommends ca-certificates openssl sudo
cp /etc/resolv.conf /tmp/cor-container-resolv.conf
umount /etc/resolv.conf
cp --remove-destination /tmp/cor-container-resolv.conf /etc/resolv.conf

groupadd -o -g "$gid" cor-ci
useradd -m -u "$uid" -g cor-ci cor-ci
printf 'cor-ci ALL=(ALL) NOPASSWD:ALL\n' > /etc/sudoers.d/cor-ci
chmod 0440 /etc/sudoers.d/cor-ci
exec runuser -u cor-ci -- "$@"
