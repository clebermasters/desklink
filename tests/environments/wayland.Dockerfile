FROM debian:trixie-slim
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    gnome-shell mutter xdg-desktop-portal xdg-desktop-portal-gnome \
    dbus-x11 dbus-user-session libgl1-mesa-dri libegl1 libgl1 \
    xwayland weston wayland-utils fonts-dejavu-core procps ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --uid 1000 --create-home tester
USER 1000:1000
ENV HOME=/home/tester XDG_CURRENT_DESKTOP=GNOME XDG_SESSION_TYPE=wayland LIBGL_ALWAYS_SOFTWARE=1
WORKDIR /workspace
USER root
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    xvfb xdotool xclip x11-utils imagemagick python3-pil wl-clipboard python3-gi \
    && rm -rf /var/lib/apt/lists/*
USER 1000:1000
