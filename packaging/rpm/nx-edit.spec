%global appid io.github.tluijken.NxEdit

Name:           nx-edit
Version:        0.1.0
Release:        1%{?dist}
Summary:        Editor for the Line 6 HX Stomp

# nx-edit is MIT; the vendored crates carry their own licenses (MIT, Apache-2.0, BSD, ISC,
# Unicode-3.0, Zlib, MPL-2.0 and others), listed in vendor/*/Cargo.toml.
License:        MIT
URL:            https://github.com/tluijken/nx-edit
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz
Source1:        %{url}/releases/download/v%{version}/%{name}-%{version}-vendor.tar.xz
# v0.1.0's source archive predates packaging/, so these come from beside the spec.
Source2:        70-nx-edit.rules
Source3:        %{appid}.desktop
Source4:        %{appid}.metainfo.xml
Source5:        %{appid}.svg

ExclusiveArch:  %{rust_arches}

BuildRequires:  cargo-rpm-macros >= 24
BuildRequires:  gcc
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib
BuildRequires:  systemd-rpm-macros

# eframe dlopens the windowing and GL libraries, so the automatic requirements miss them.
Requires:       libxkbcommon.so.0()(64bit)
Requires:       libwayland-client.so.0()(64bit)
Requires:       libwayland-egl.so.1()(64bit)
Requires:       libEGL.so.1()(64bit)
Requires:       libGL.so.1()(64bit)
Requires:       hicolor-icon-theme
Recommends:     vulkan-loader
Recommends:     libX11
Recommends:     libXcursor
Recommends:     libXrandr
Recommends:     libXi
Recommends:     libxkbcommon-x11
Recommends:     /usr/bin/7z

%description
A native editor for the Line 6 HX Stomp. It speaks the pedal's USB protocol
directly. Block and parameter names are imported once from the user's own
HX Edit installer.

%prep
%autosetup -n %{name}-%{version} -a 1
%cargo_prep -v vendor

%build
%cargo_build -- -p nx-app

%install
install -Dm755 target/rpm/nx-edit %{buildroot}%{_bindir}/nx-edit
install -Dm644 %{SOURCE2} %{buildroot}%{_udevrulesdir}/70-nx-edit.rules
install -Dm644 %{SOURCE3} %{buildroot}%{_datadir}/applications/%{appid}.desktop
install -Dm644 %{SOURCE4} %{buildroot}%{_metainfodir}/%{appid}.metainfo.xml
install -Dm644 %{SOURCE5} %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/%{appid}.svg

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/%{appid}.desktop
appstream-util validate-relax --nonet %{buildroot}%{_metainfodir}/%{appid}.metainfo.xml
%cargo_test -- --workspace

%files
%license LICENSE
%doc README.md
%{_bindir}/nx-edit
%{_udevrulesdir}/70-nx-edit.rules
%{_datadir}/applications/%{appid}.desktop
%{_metainfodir}/%{appid}.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/%{appid}.svg

%changelog
* Tue Nov 17 2026 Thomas Luijken <github@luijken.dev> - 0.1.0-1
- First package
