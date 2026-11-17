#!/usr/bin/env bash
# Copy the artwork the GUI needs from the vendor installer's res/ folder (extracted with
# `innoextract -e <setup.exe>`) to ~/.local/share/mouse-config/assets under ASCII names.
# The artwork belongs to Ardor Gaming: keep it local, do not commit or redistribute it.
set -euo pipefail
src=${1:?usage: install-assets.sh <path to the vendor res folder>}
dst=${XDG_DATA_HOME:-$HOME/.local/share}/mouse-config/assets
mkdir -p "$dst"
declare -A map=(
	[bg.png]="2按钮界面/主界面背景.png"
	[nav.png]="2按钮界面/导航栏默认.png"
	[nav_on.png]="2按钮界面/导航栏选择.png"
	[nav_hover.png]="2按钮界面/导航栏鼠标进入.png"
	[mouse.png]="2按钮界面/设备1.png"
	[key.png]="2按钮界面/鼠标主图按键.png"
	[key_down.png]="2按钮界面/鼠标主图按键按下.png"
	[key_hover.png]="2按钮界面/鼠标主图按键鼠标进入.png"
	[battery_full.png]="2按钮界面/满电量.png"
	[battery_empty.png]="2按钮界面/空电量.png"
	[battery_low.png]="2按钮界面/低电量.png"
	[panel.png]="2按钮界面/按键功能背景.png"
	[reset.png]="2按钮界面/恢复默认按键.png"
	[reset_hover.png]="2按钮界面/恢复默认按键鼠标进入.png"
	[sys.png]="2按钮界面/系统鼠标按键.png"
	[sys_down.png]="2按钮界面/系统鼠标按键按下.png"
	[dpi_off.png]="3传感器界面/DPI按键未选择.png"
	[dpi_on.png]="3传感器界面/DPI按键选择.png"
	[rate_off.png]="3传感器界面/回报率按键未选择.png"
	[rate_on.png]="3传感器界面/回报率按键选择.png"
	[dpi_cur.png]="3传感器界面/当前DPI档位.png"
	[dpi_cur_hover.png]="3传感器界面/当前DPI档位鼠标进入.png"
	[home_on.png]="1首页/设备1在线.png"
	[home_off.png]="1首页/设备1不在线.png"
	[wired.png]="1首页/有线连接.png"
	[wireless.png]="1首页/无线连接.png"
	[color_bar.png]="5灯光界面/颜色条.png"
)
for name in "${!map[@]}"; do
	cp "$src/${map[$name]}" "$dst/$name"
done
echo "installed ${#map[@]} images to $dst"
