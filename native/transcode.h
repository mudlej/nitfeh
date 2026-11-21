#ifndef NITFEH_TRANSCODE_H
#define NITFEH_TRANSCODE_H

#include <stdint.h>

/* Return codes. Negative values are failures; `err` then holds a message. */
#define NITFEH_OK 0
#define NITFEH_CANCELLED 1
#define NITFEH_ERROR -1
#define NITFEH_NO_AUDIO -2

typedef struct NitfehInfo {
    int64_t duration_us; /* 0 when unknown */
    int has_audio;
    int sample_rate;
    int channels;
    char codec[32];
} NitfehInfo;

typedef struct NitfehOptions {
    int bitrate;     /* bits per second */
    int sample_rate; /* 8000, 12000, 16000, 24000 or 48000 */
    int frame_ms;    /* Opus frame duration */
} NitfehOptions;

typedef struct NitfehResult {
    int64_t duration_us; /* audio written */
    int decode_errors;   /* corrupt packets that were skipped */
    int truncated;       /* the input ended with a read error rather than a clean end */
} NitfehResult;

/* Called about once per second of audio. A non-zero return cancels the job. */
typedef int (*nitfeh_progress_fn)(void *opaque, int64_t done_us, int64_t total_us);

/* Paths are UTF-8 on every platform. */
void nitfeh_init(int verbose);
int nitfeh_probe(const char *path, NitfehInfo *info, char *err, int err_len);
int nitfeh_transcode(const char *in_path, const char *out_path, const NitfehOptions *options,
                     nitfeh_progress_fn progress, void *opaque, NitfehResult *result, char *err,
                     int err_len);

#endif
