#!/bin/sh
set -eu

uid=$1
gid=$2
distribution=$3
release=$4
shift 4

apt_update() {
    apt-get -o Acquire::Retries=2 -o Acquire::Languages=none -o APT::Update::Error-Mode=any update
}

apt_install() {
    apt-get -o Acquire::Retries=2 install -y --no-install-recommends "$@"
}

. /etc/os-release
case "$ID" in
    debian|ubuntu)
        export DEBIAN_FRONTEND=noninteractive
        apt_update
        apt_install ca-certificates mount openssl sudo
        case "$distribution" in
            linuxmint)
                dpkg-deb -x /work/release-keyring.deb /tmp/cor-mint-keyring
                install -m 0644 /tmp/cor-mint-keyring/usr/share/keyrings/linuxmint-keyring.gpg \
                    /usr/share/keyrings/cor-linuxmint.asc
                dpkg -i /work/base-files.deb
                . /etc/os-release
                test "$ID:$VERSION_ID" = "linuxmint:$release"
                printf 'deb [signed-by=/usr/share/keyrings/cor-linuxmint.asc] https://fastly.linuxmint.io %s main upstream import backport\n' \
                    "$VERSION_CODENAME" > /etc/apt/sources.list.d/cor-linuxmint.list
                apt-mark hold base-files
                apt_update
                ;;
            pop)
                dpkg -i /work/release-keyring.deb
                printf 'Types: deb\nURIs: https://apt.pop-os.org/release\nSuites: %s\nComponents: main\nSigned-By: %s\n' \
                    "$VERSION_CODENAME" /usr/share/keyrings/pop-archive-keyring.gpg \
                    > /etc/apt/sources.list.d/pop-os-release.sources
                if [ -f /etc/apt/sources.list ]; then
                    cp /etc/apt/sources.list /tmp/cor-ubuntu-sources.list
                fi
                apt_update
                apt_install lsb-release pop-default-settings
                . /etc/os-release
                test "$ID:$VERSION_ID" = "pop:$release"
                if [ -f /tmp/cor-ubuntu-sources.list ]; then
                    mv /tmp/cor-ubuntu-sources.list /etc/apt/sources.list
                fi
                rm -f /etc/apt/sources.list.d/system.sources /etc/apt/sources.list.d/pop-os-apps.sources
                ;;
            zorin)
                install -m 0644 /work/patches.asc /usr/share/keyrings/cor-zorin-patches.asc
                printf 'deb [signed-by=/usr/share/keyrings/cor-zorin-patches.asc] https://ppa.launchpadcontent.net/zorinos/patches/ubuntu %s main\n' \
                    "$VERSION_CODENAME" > /etc/apt/sources.list.d/cor-zorin-patches.list
                dpkg -i /work/base-files.deb
                . /etc/os-release
                test "$ID:$VERSION" = "zorin:$release"
                apt-mark hold base-files
                apt_update
                ;;
        esac
        ;;
    fedora)
        dnf -y --setopt=install_weak_deps=False install ca-certificates shadow-utils sudo util-linux
        ;;
    opensuse-leap|opensuse-tumbleweed)
        if [ "$ID" = opensuse-tumbleweed ]; then
            PYTHONPATH=/work/scripts /work/python/bin/python3 -c \
                'from linux_native_guest import refresh_tumbleweed_container; refresh_tumbleweed_container()'
        else
            zypper --non-interactive refresh
        fi
        zypper --non-interactive install --no-recommends ca-certificates shadow sudo util-linux
        ;;
    arch)
        PYTHONPATH=/work/scripts /work/python/bin/python3 -c \
            'from linux_native_guest import arch_snapshot_mirrorlist; print(arch_snapshot_mirrorlist(), end="")' \
            > /etc/pacman.d/mirrorlist
        pacman-key --init
        pacman-key --populate archlinux
        pacman -Syyu --noconfirm --needed ca-certificates shadow sudo util-linux
        ;;
    *)
        printf 'Unsupported compatibility container: %s\n' "$ID" >&2
        exit 1
        ;;
esac

cp /etc/resolv.conf /tmp/cor-container-resolv.conf
umount /etc/resolv.conf
cp --remove-destination /tmp/cor-container-resolv.conf /etc/resolv.conf
groupadd -o -g "$gid" cor-ci
useradd -m -u "$uid" -g cor-ci cor-ci
printf 'cor-ci ALL=(ALL) NOPASSWD:ALL\n' > /etc/sudoers.d/cor-ci
chmod 0440 /etc/sudoers.d/cor-ci
exec runuser -u cor-ci -- "$@"
