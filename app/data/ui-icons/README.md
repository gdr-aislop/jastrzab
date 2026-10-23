# Bundled UI icons

The symbolic icons the app names (`location-services-*`, `find-location`, `network-*`, `go-next`,
`edit-copy`, `document-edit` (the pencil of every entry row, named by libadwaita), `process-stop`, `open-menu`, `list-add`, `help-about`, `window-close`), compiled into the
binary (`../jastrzab.gresource.xml`) and added to the icon search path at start-up. Without them
the app shows "missing image" icons wherever the system's icon theme lacks them, which is what
happens in a Flatpak whose host uses a theme the sandbox does not have.

They are copies of the Adwaita icon theme's symbolic icons (GNOME project,
<https://gitlab.gnome.org/GNOME/adwaita-icon-theme>), taken from version 43 (the one on PureOS
Crimson). Licence: LGPL-3.0-or-later or CC-BY-SA-3.0, at your option; the LGPL is compatible with the
GPL-3.0-or-later of the rest of the project.

The tests in `src/ui/icons.rs` fail if the code names a `-symbolic` icon that is not here.
