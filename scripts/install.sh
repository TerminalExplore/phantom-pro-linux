#!/usr/bin/env bash
# Build the CLI and the GUI and make the GUI launchable: ~/.local/bin links, a launcher script,
# an icon and a .desktop entry (shows up in rofi/drun and application menus).
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
bin=${XDG_BIN_HOME:-$HOME/.local/bin}
data=${XDG_DATA_HOME:-$HOME/.local/share}
mkdir -p "$bin" "$data/applications" "$data/icons/hicolor/256x256/apps"

cargo build --release --features gui --manifest-path "$repo/Cargo.toml"
# links, so a rebuild is picked up without installing again
ln -sf "$repo/target/release/mouse-config" "$bin/mouse-config"
ln -sf "$repo/target/release/mouse-config-gui" "$bin/mouse-config-gui"

cat >"$bin/phantom-mouse" <<LAUNCH
#!/bin/sh
# Open the Phantom Pro V2 settings window. Under bspwm it opens floating at its own size
# (a one-shot rule, so bspwmrc stays untouched).
command -v bspc >/dev/null 2>&1 && bspc rule -a mouse-config --one-shot state=floating center=on 2>/dev/null
exec "$bin/mouse-config-gui" "\$@"
LAUNCH
chmod +x "$bin/phantom-mouse"

# icon: the mouse photo from the vendor artwork when it is installed, else the theme's mouse icon
icon=input-mouse
photo=$data/mouse-config/assets/mouse.png
if [ -f "$photo" ] && python3 - "$photo" "$data/icons/hicolor/256x256/apps/mouse-config.png" <<'PY' 2>/dev/null
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGBA")
side = max(im.size)
canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
canvas.paste(im, ((side - im.size[0]) // 2, (side - im.size[1]) // 2))
canvas.resize((256, 256), Image.LANCZOS).save(sys.argv[2])
PY
then
	icon=mouse-config
fi

cat >"$data/applications/mouse-config.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Phantom Pro V2
GenericName=Mouse settings
Comment=Ardor Phantom Pro V2: DPI, buttons, macros, lighting
Exec=$bin/phantom-mouse
Icon=$icon
Terminal=false
Categories=Settings;HardwareSettings;
Keywords=mouse;ardor;phantom;dpi;мышь;
StartupWMClass=mouse-config
DESKTOP
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$data/icons/hicolor" 2>/dev/null || true
echo "installed: $bin/phantom-mouse, $data/applications/mouse-config.desktop (icon: $icon)"
