// libobs's util/util_uint64.h divides with MSVC's `_udiv128` when `_MSC_VER` says MSVC 2019 or
// newer, which libclang also claims when bindgen reads the headers for an MSVC target, but
// clang's intrin.h does not declare it. bindgen only parses that inline function, so its
// declaration is all clang needs.
#if defined(_MSC_VER) && defined(_M_X64) && defined(__clang__)
unsigned __int64 _udiv128(unsigned __int64 high, unsigned __int64 low, unsigned __int64 divisor,
                          unsigned __int64 *remainder);
#endif
#include <obs.h>
#include <obs-output.h>
#include <obs-properties.h>
#include <obs-data.h>
#include <util/base.h>
#include <util/platform.h>
#if defined(__linux__)
#include <obs-nix-platform.h>
#endif
