#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Build the macOS app's image and archive libraries from source (D5,
# DD-34, DD-44): a minimal libvips (no ImageMagick, poppler, cairo or
# libimagequant; libheif without x265) and libarchive, with their
# libraries, for arm64 and macOS 14, into <native-dir>/prefix with
# @rpath install names. Homebrew provides build tools only: pkg-config
# sees only the prefix, and CMake ignores Homebrew's folders.
#
#   build-libs.sh [native-dir]      (default target/native)
# shellcheck source=lib.sh
. "$(cd "$(dirname "$0")" && pwd)/lib.sh"

# macOS's own zlib has no pkg-config file, and libpng requires one.
printf 'Name: zlib\nDescription: the system zlib\nVersion: 1.2.12\nLibs: -lz\nCflags:\n' > "$P/lib/pkgconfig/zlib.pc"

step libffi auto_build --disable-docs
# No JIT: under the hardened runtime it would need the allow-jit entitlement.
step pcre2 cmake_build -DPCRE2_BUILD_TESTS=OFF -DPCRE2_BUILD_PCRE2GREP=OFF -DPCRE2_SUPPORT_JIT=OFF -DPCRE2_BUILD_PCRE2_8=ON
# glib's gettext stand-in, proxy-libintl, is one of its meson subprojects:
# meson finds the pinned tarball in its package cache instead of
# downloading it.
glib_build() {
  local d=$1 intl; shift
  intl=$(fetch proxy-libintl)
  licences proxy-libintl "$intl"
  mkdir -p "$d/subprojects/packagecache"
  cp "$SRC/proxy-libintl-$(version_of proxy-libintl).tar.gz" "$d/subprojects/packagecache/"
  meson_build "$d" "$@"
}
step glib glib_build -Dtests=false -Dintrospection=disabled -Dnls=disabled -Dman-pages=disabled \
  -Ddocumentation=false -Dsysprof=disabled -Dglib_debug=disabled -Dlibmount=disabled -Dselinux=disabled -Dxattr=false
step expat cmake_build -DEXPAT_BUILD_TESTS=OFF -DEXPAT_BUILD_EXAMPLES=OFF -DEXPAT_BUILD_TOOLS=OFF -DEXPAT_BUILD_DOCS=OFF
step libjpeg-turbo cmake_build -DENABLE_STATIC=OFF -DWITH_TURBOJPEG=OFF
step libpng cmake_build -DPNG_STATIC=OFF -DPNG_TESTS=OFF -DPNG_TOOLS=OFF -DPNG_FRAMEWORK=OFF
step libwebp cmake_build -DWEBP_BUILD_ANIM_UTILS=OFF -DWEBP_BUILD_CWEBP=OFF -DWEBP_BUILD_DWEBP=OFF \
  -DWEBP_BUILD_GIF2WEBP=OFF -DWEBP_BUILD_IMG2WEBP=OFF -DWEBP_BUILD_VWEBP=OFF -DWEBP_BUILD_WEBPINFO=OFF \
  -DWEBP_BUILD_WEBPMUX=OFF -DWEBP_BUILD_EXTRAS=OFF
step libtiff cmake_build -Dtiff-tools=OFF -Dtiff-tests=OFF -Dtiff-contrib=OFF -Dtiff-docs=OFF \
  -Djbig=OFF -Dlerc=OFF -Dlzma=OFF -Dzstd=OFF -Dwebp=OFF -Dlibdeflate=OFF
step lcms2 auto_build
step libexif auto_build --disable-docs --disable-nls
step highway cmake_build -DHWY_ENABLE_TESTS=OFF -DHWY_ENABLE_EXAMPLES=OFF -DHWY_ENABLE_CONTRIB=OFF -DBUILD_TESTING=OFF
step libde265 cmake_build -DENABLE_SDL=OFF -DENABLE_DECODER=OFF -DENABLE_ENCODER=OFF
step dav1d meson_build -Denable_tools=false -Denable_tests=false
# AV1 encoding only: dav1d decodes.
step libaom cmake_build -DENABLE_DOCS=0 -DENABLE_EXAMPLES=0 -DENABLE_TESTS=0 -DENABLE_TOOLS=0 -DCONFIG_AV1_DECODER=0
# No x265 (DD-34); codecs built in, so there's no plugin folder to bundle.
step libheif cmake_build -DWITH_X265=OFF -DWITH_LIBDE265=ON -DWITH_DAV1D=ON -DWITH_AOM_ENCODER=ON \
  -DWITH_AOM_DECODER=OFF -DWITH_EXAMPLES=OFF -DBUILD_TESTING=OFF -DWITH_GDK_PIXBUF=OFF -DWITH_RAV1E=OFF \
  -DWITH_SvtEnc=OFF -DWITH_KVAZAAR=OFF -DWITH_OpenJPEG_DECODER=OFF -DWITH_FFMPEG_DECODER=OFF \
  -DWITH_JPEG_DECODER=OFF -DWITH_JPEG_ENCODER=OFF -DWITH_OPENJPH_ENCODER=OFF -DENABLE_PLUGIN_LOADING=OFF
step cgif meson_build -Dtests=false
step libvips meson_build -Dmodules=disabled -Dintrospection=disabled -Dexamples=false -Dcplusplus=false \
  -Dmagick=disabled -Dpoppler=disabled -Dpdfium=disabled -Drsvg=disabled -Dimagequant=disabled -Dquantizr=disabled \
  -Dfftw=disabled -Dmatio=disabled -Dnifti=disabled -Dopenexr=disabled -Dopenjpeg=disabled -Dopenslide=disabled \
  -Dorc=disabled -Dpangocairo=disabled -Dcfitsio=disabled -Djpeg-xl=disabled -Dspng=disabled -Draw=disabled \
  -Duhdr=disabled -Darchive=disabled -Dfontconfig=disabled \
  -Dpng=enabled -Dheif=enabled -Dexif=enabled -Dlcms=enabled -Dtiff=enabled -Dwebp=enabled -Djpeg=enabled \
  -Dcgif=enabled -Dhighway=enabled -Dzlib=enabled
step libarchive cmake_build -DENABLE_TEST=OFF -DENABLE_TAR=OFF -DENABLE_CPIO=OFF -DENABLE_CAT=OFF \
  -DENABLE_UNZIP=OFF -DENABLE_OPENSSL=OFF -DENABLE_LZMA=OFF -DENABLE_ZSTD=OFF -DENABLE_LZ4=OFF \
  -DENABLE_LIBXML2=OFF -DENABLE_EXPAT=OFF -DENABLE_MBEDTLS=OFF -DENABLE_NETTLE=OFF -DENABLE_LIBB2=OFF

fix_names
rm -rf "${BUILD:?}"
echo "=== built"; ls "$P/lib"/*.dylib | xargs -n1 basename
