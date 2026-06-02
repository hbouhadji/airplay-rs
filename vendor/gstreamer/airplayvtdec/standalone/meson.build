project('airplayvtdec', ['c', 'objc'],
  version: '0.1.0',
  default_options: ['c_std=gnu11'])

cc = meson.get_compiler('c')
objc = meson.get_compiler('objc')

gst_dep = dependency('gstreamer-1.0', version: '>=1.28')
gstbase_dep = dependency('gstreamer-base-1.0', version: '>=1.28')
gstvideo_dep = dependency('gstreamer-video-1.0', version: '>=1.28')
gstgl_dep = dependency('gstreamer-gl-1.0', version: '>=1.28')
gstcodecparsers_dep = dependency('gstreamer-codecparsers-1.0', version: '>=1.28')
gstpbutils_dep = dependency('gstreamer-pbutils-1.0', version: '>=1.28')

frameworks = [
  dependency('AVFoundation'),
  dependency('CoreFoundation'),
  dependency('CoreMedia'),
  dependency('CoreVideo'),
  dependency('VideoToolbox'),
  dependency('Cocoa'),
  dependency('IOSurface'),
  dependency('appleframeworks', modules: ['OpenGL']),
]

sources = [
  'coremediabuffer.c',
  'corevideobuffer.c',
  'corevideomemory.c',
  'glcontexthelper.c',
  'helpers.m',
  'iosurfaceglmemory.c',
  'plugin.m',
  'videotexturecache.m',
  'videotexturecache-gl.m',
  'vtdec.c',
  'vtutil.c',
]

common_args = [
  '-DVERSION="@0@"'.format(meson.project_version()),
  '-DPACKAGE="airplayvtdec"',
  '-DGST_PACKAGE_NAME="airplay-rs"',
  '-DGST_PACKAGE_ORIGIN="https://example.invalid"',
  '-DGST_USE_UNSTABLE_API',
  '-Dgst_core_media_buffer_new=gst_airplay_core_media_buffer_new',
  '-Dgst_core_media_buffer_get_type=gst_airplay_core_media_buffer_get_type',
  '-Dgst_core_media_buffer_get_pixel_buffer=gst_airplay_core_media_buffer_get_pixel_buffer',
  '-Dgst_core_video_buffer_new=gst_airplay_core_video_buffer_new',
  '-Dgst_core_video_buffer_get_type=gst_airplay_core_video_buffer_get_type',
]

objc_args = common_args + ['-fobjc-arc']

library('gstairplayvtdec',
  sources,
  c_args: common_args,
  objc_args: objc_args,
  dependencies: [
    gst_dep,
    gstbase_dep,
    gstvideo_dep,
    gstgl_dep,
    gstcodecparsers_dep,
    gstpbutils_dep,
  ] + frameworks,
  name_prefix: 'lib',
  install: false)
