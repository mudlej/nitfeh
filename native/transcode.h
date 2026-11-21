#ifndef ISTMA_TRANSCODE_H
#define ISTMA_TRANSCODE_H

#include <stdint.h>

/* Return codes. Negative values are failures; `err` then holds a message. */
#define ISTMA_OK 0
#define ISTMA_CANCELLED 1
#define ISTMA_ERROR -1
#define ISTMA_NO_AUDIO -2

typedef struct IstmaInfo {
    int64_t duration_us; /* 0 when unknown */
    int has_audio;
    int sample_rate;
    int channels;
    char codec[32];
} IstmaInfo;

typedef struct IstmaOptions {
    int bitrate;     /* bits per second */
    int sample_rate; /* 8000, 12000, 16000, 24000 or 48000 */
    int frame_ms;    /* Opus frame duration */
} IstmaOptions;

typedef struct IstmaResult {
    int64_t duration_us; /* audio written */
    int decode_errors;   /* corrupt packets that were skipped */
    int truncated;       /* the input ended with a read error rather than a clean end */
} IstmaResult;

/* Called about once per second of audio. A non-zero return cancels the job. */
typedef int (*istma_progress_fn)(void *opaque, int64_t done_us, int64_t total_us);

/* Paths are UTF-8 on every platform. */
void istma_init(int verbose);
int istma_probe(const char *path, IstmaInfo *info, char *err, int err_len);
int istma_transcode(const char *in_path, const char *out_path, const IstmaOptions *options,
                    istma_progress_fn progress, void *opaque, IstmaResult *result, char *err,
                    int err_len);

#endif
