/*
 * Local GStreamer plugin for AirPlay low-latency VideoToolbox decoding.
 *
 * This registers a renamed copy of applemedia's vtdec as airplayvtdec and
 * airplayvtdec_hw. The implementation is intentionally kept separate from the
 * system applemedia plugin so the application can opt in explicitly.
 */

#ifdef HAVE_CONFIG_H
#include "config.h"
#endif

#include <TargetConditionals.h>
#include <Foundation/Foundation.h>

#include "corevideomemory.h"
#include "vtdec.h"

#if TARGET_OS_OSX
static void
enable_mt_mode (void)
{
  NSThread *th = [[NSThread alloc] init];
  [th start];
  g_assert ([NSThread isMultiThreaded]);
}
#endif

static gboolean
plugin_init (GstPlugin * plugin)
{
  gboolean res = FALSE;

  gst_apple_core_video_memory_init ();

#if TARGET_OS_OSX
  enable_mt_mode ();
#endif

  res |= gst_vtdec_register_elements (plugin);
  return res;
}

GST_PLUGIN_DEFINE (GST_VERSION_MAJOR,
    GST_VERSION_MINOR,
    airplayvtdec,
    "Low-latency AirPlay VideoToolbox decoder",
    plugin_init, VERSION, "LGPL", "airplay-rs", "https://example.invalid")
