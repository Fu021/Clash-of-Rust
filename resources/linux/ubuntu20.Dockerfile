ARG BASE_IMAGE
FROM ${BASE_IMAGE}
ARG USER_ID=1001
ARG GROUP_ID=1001
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates openssl build-essential binutils git pkg-config cmake ninja-build perl \
    libglib2.0-dev libfontconfig1-dev libfreetype6-dev libxkbcommon-dev \
    libwayland-dev libx11-dev libx11-xcb-dev gsettings-desktop-schemas \
    dbus dbus-x11 libxkbcommon-x11-0 libxcursor1 libxrandr2 libxi6 libxinerama1 \
    libegl1 iproute2 util-linux libcap2-bin systemd sudo dpkg-dev xvfb x11-utils fonts-wqy-microhei \
    libssl1.1 zlib1g libffi7 libbz2-1.0 liblzma5 libsqlite3-0 libreadline8 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd -o -g ${GROUP_ID} builder \
    && useradd -o -u ${USER_ID} -g ${GROUP_ID} -m -s /bin/bash builder \
    && printf 'builder ALL=(ALL) NOPASSWD:ALL\n' > /etc/sudoers.d/cor-builder \
    && chmod 0440 /etc/sudoers.d/cor-builder
COPY python.tar.gz /tmp/python.tar.gz
RUN mkdir /opt/python && tar xzf /tmp/python.tar.gz -C /opt/python --strip-components=1 \
    && rm /tmp/python.tar.gz && /opt/python/bin/python3 --version
ENV PATH="/opt/python/bin:${PATH}"
