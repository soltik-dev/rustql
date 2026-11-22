# RPM for COPR (dukerth/rustql). A template: `scripts/release.py spec`
# fills in the @…@ fields (version, the licenses of the vendored crates and
# the changelog entry) and distribute.yml builds the SRPM with it.

# Release builds are stripped (Cargo.toml): there is no debug info to package
%global debug_package %{nil}

Name:           rustql
Version:        @VERSION@
Release:        @RELEASE@%{?dist}
Summary:        Fast, native client for MySQL, MariaDB and PostgreSQL

# RustQL is GPL-3.0-or-later and its fonts OFL-1.1; the rest are the licenses
# of the Rust crates compiled into the binary
License:        @LICENSE@
URL:            https://rustql.com
Source0:        https://github.com/soltik-dev/rustql/releases/download/v%{version}/rustql-%{version}-vendored.tar.xz

BuildRequires:  cargo >= 1.95
BuildRequires:  rust >= 1.95
BuildRequires:  gcc
BuildRequires:  pkgconfig(zlib)
BuildRequires:  desktop-file-utils
BuildRequires:  appstream

%description
RustQL is a desktop client for MySQL, MariaDB and PostgreSQL written in Rust.
It starts instantly, stays responsive with large tables and keeps your
passwords in the system keyring.

%prep
%autosetup -n rustql-%{version}

%build
# The plain version in the status bar, without a commit (see build.rs)
export RUSTQL_RELEASE=1
cargo build --release --offline --locked

%install
install -Dpm 0755 target/release/rustql %{buildroot}%{_bindir}/rustql
install -Dpm 0644 packaging/linux/com.rustql.RustQL.desktop \
    %{buildroot}%{_datadir}/applications/com.rustql.RustQL.desktop
install -Dpm 0644 packaging/linux/com.rustql.RustQL.metainfo.xml \
    %{buildroot}%{_metainfodir}/com.rustql.RustQL.metainfo.xml
install -Dpm 0644 assets/icon/icon.png \
    %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/com.rustql.RustQL.png

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/com.rustql.RustQL.desktop
appstreamcli validate --no-net %{buildroot}%{_metainfodir}/com.rustql.RustQL.metainfo.xml

%files
%license LICENSE assets/fonts/OFL-*.txt
%doc README.md CHANGELOG.md
%{_bindir}/rustql
%{_datadir}/applications/com.rustql.RustQL.desktop
%{_metainfodir}/com.rustql.RustQL.metainfo.xml
%{_datadir}/icons/hicolor/256x256/apps/com.rustql.RustQL.png

%changelog
@CHANGELOG@
