# SPDX-License-Identifier: GPL-3.0-or-later
# Every source the macOS app's native libraries and FFmpeg are built from
# (FFmpeg's codecs last; x264 has no releases, so it's its stable branch,
# pinned by commit),
# one per line: name, version, URL, SHA-256. lib.sh's fetch refuses a
# download whose checksum differs. Changing a line here changes the release
# workflow's cache key, so the prefix is rebuilt. proxy-libintl is the
# gettext stand-in glib builds as a meson subproject; it's fetched here, so
# meson itself never downloads anything.
# shellcheck disable=SC2034
SOURCES="
libffi 3.8.0 https://github.com/libffi/libffi/releases/download/v3.8.0/libffi-3.8.0.tar.gz 7da3e2d9a171eb0a038f592ecad3ff2bb2550f3496d87b3b29ad0cf4430c0db4
pcre2 10.49 https://github.com/PCRE2Project/pcre2/releases/download/pcre2-10.49/pcre2-10.49.tar.bz2 53c156e1ba416a20da8e65395daa132da0d80e76910424caca3fcdae7831d384
proxy-libintl 0.5 https://github.com/frida/proxy-libintl/archive/refs/tags/0.5.tar.gz f7a1cbd7579baaf575c66f9d99fb6295e9b0684a28b095967cfda17857595303
glib 2.90.1 https://download.gnome.org/sources/glib/2.90/glib-2.90.1.tar.xz 93c941aa17d5eb1d53fe838365f29a8b4e539c222a256d974ec8f30fc413e396
expat 2.10.0 https://github.com/libexpat/libexpat/releases/download/R_2_10_0/expat-2.10.0.tar.xz c624ba2998fc65df1163a49641ff3f9ab73138b11641d92c86bee0fb01b1bc7f
libjpeg-turbo 3.2.1 https://github.com/libjpeg-turbo/libjpeg-turbo/releases/download/3.2.1/libjpeg-turbo-3.2.1.tar.gz c27ad72a68da151177929aa969ae773a2da5ac2af9ce946079314fdac8cece7f
libpng 1.6.59 https://github.com/pnggroup/libpng/archive/refs/tags/v1.6.59.tar.gz 2540302a1844ad2b2b501977abecfa850f265f97b78f065a712ab4074a89f5b5
libwebp 1.6.0 https://github.com/webmproject/libwebp/archive/refs/tags/v1.6.0.tar.gz 93a852c2b3efafee3723efd4636de855b46f9fe1efddd607e1f42f60fc8f2136
libtiff 4.7.2 https://download.osgeo.org/libtiff/tiff-4.7.2.tar.xz 4996f0c4f93094719b1ca5c6279b20e588773ba8a247533e486416fb662ddb88
lcms2 2.20 https://github.com/mm2/Little-CMS/releases/download/lcms2.20/lcms2-2.20.tar.gz 18ff5504fa9cc8a2b1f3591180d90ead16c8a2ea78e530523b46eab70e1ca6b9
libexif 0.6.26 https://github.com/libexif/libexif/releases/download/v0.6.26/libexif-0.6.26.tar.xz 4a055ed6575e61ca46c3172be3c753cc16c9becd0f99ec71d58dd0e471476c0c
highway 1.4.0 https://github.com/google/highway/archive/refs/tags/1.4.0.tar.gz e72241ac9524bb653ae52ced768b508045d4438726a303f10181a38f764a453c
libde265 1.1.3 https://github.com/strukturag/libde265/releases/download/v1.1.3/libde265-1.1.3.tar.gz 554228bd17788c99a7e63b37ab5634722190e6e2bf60c1dcb01cef328e133905
dav1d 1.5.4 https://code.videolan.org/videolan/dav1d/-/archive/1.5.4/dav1d-1.5.4.tar.bz2 2abfb0c89212e6e4733a54e0ae509ec00a5b845a6360946f918806e14aedb011
libaom 3.15.3 https://storage.googleapis.com/aom-releases/libaom-3.15.3.tar.gz 12c01dc731ed7b5a999e28120b45bf58a002ba3435a4ba2cfbc1e084a27c9c04
libheif 1.23.7 https://github.com/strukturag/libheif/releases/download/v1.23.7/libheif-1.23.7.tar.gz f7483e818503ea6224c8863e4a56ac02c7089021a98472c811c12aa19faf2475
cgif 0.5.4 https://github.com/dloebl/cgif/archive/refs/tags/v0.5.4.tar.gz 83a70a15bc2da41f081a44ebc58ee48e2e1d524a6d3fdb4a24064afa08d5ad4d
libvips 8.18.7 https://github.com/libvips/libvips/releases/download/v8.18.7/vips-8.18.7.tar.xz 5baaead3b0bb20ffdb9e9ff09aa9fda08620923df77b63b436654cb5e0b3bf94
libarchive 3.8.11 https://github.com/libarchive/libarchive/releases/download/v3.8.11/libarchive-3.8.11.tar.xz a3dc0872b1b39da34dc18ef1b944f0f11ffd37425d70ae04be679279f1cf43d2
ffmpeg 9.0.2 https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz 8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e
x264 b35605ace3ddf7c1a5d67a2eb553f034aef41d55 https://code.videolan.org/videolan/x264/-/archive/b35605ace3ddf7c1a5d67a2eb553f034aef41d55/x264-b35605ace3ddf7c1a5d67a2eb553f034aef41d55.tar.bz2 6eeb82934e69fd51e043bd8c5b0d152839638d1ce7aa4eea65a3fedcf83ff224
x265 4.2 https://download.videolan.org/videolan/x265/x265_4.2.tar.gz 40b1ea0453e0309f0eba934e0ddf533f8f6295966679e8894e8f1c1c8d5e1210
libvpx 1.17.0 https://github.com/webmproject/libvpx/archive/refs/tags/v1.17.0.tar.gz 1020f184046187baa2985dbde38e0691f49c44088bca7a1842b0236c6081dc0a
opus 1.6.1 https://downloads.xiph.org/releases/opus/opus-1.6.1.tar.gz 6ffcb593207be92584df15b32466ed64bbec99109f007c82205f0194572411a1
lame 3.100 https://downloads.sourceforge.net/project/lame/lame/3.100/lame-3.100.tar.gz ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e
"
