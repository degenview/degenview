# dmgbuild settings for the macOS disk image. tools/make-dmg.sh runs
#
#   dmgbuild -s packaging/macos/dmg-settings.py -D app=... -D background=... -D icon=... DegenView out.dmg
#
# The window shows DegenView.app and an Applications shortcut for the user to drag it onto. The icon
# positions are the centres of the icons, in the same points tools/make-dmg-background.py draws its
# arrow between; change them together.

defines = globals().get("defines", {})
app = defines["app"]

# Same compression the shipped zips' contents would get; the file is signed and notarized after this.
format = "UDZO"
filesystem = "HFS+"

# The volume (and so the window title) is `DegenView`; the volume icon is the app icon.
files = [app]
symlinks = {"Applications": "/Applications"}
icon = defines["icon"]
background = defines["background"]

# The window frame, title bar included. Finder shows the user's own status/path/tab bars whatever the
# image says, so keep everything that matters inside the top ~256 points (see the art script).
window_rect = ((200, 120), (660, 380))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False

icon_size = 128
text_size = 13
arrange_by = None
label_pos = "bottom"
icon_locations = {
    "DegenView.app": (180, 125),
    "Applications": (480, 125),
}
