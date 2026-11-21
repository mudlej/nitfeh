/* The audio pipeline: demux, decode, downmix and resample, encode with libopus, write Ogg Opus.
 * It is what `ffmpeg -i IN -vn -ac 1 -ar 16000 -c:a libopus -b:a N -application voip -frame_duration 60 OUT`
 * does, on the same libraries. */
#include "transcode.h"

#include <stdio.h>
#include <string.h>

#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/audio_fifo.h>
#include <libavutil/avstring.h>
#include <libavutil/channel_layout.h>
#include <libavutil/opt.h>
#include <libswresample/swresample.h>

typedef struct Job {
    AVFormatContext *in, *out;
    AVCodecContext *dec, *enc;
    SwrContext *swr;
    AVAudioFifo *fifo;
    AVPacket *pkt;
    AVFrame *frame;
    AVStream *out_stream;
    int stream;

    /* What the resampler is currently set up to read; decoders may change it mid-stream. */
    int in_rate;
    enum AVSampleFormat in_fmt;
    AVChannelLayout in_layout;

    uint8_t *buf;
    int buf_samples;
    int64_t samples; /* sent to the encoder */
} Job;

static void fail(char *err, int err_len, const char *what, int code) {
    char reason[128] = "";
    if (!err || err_len <= 0)
        return;
    if (code) {
        av_strerror(code, reason, sizeof(reason));
        snprintf(err, err_len, "%s: %s", what, reason);
    } else {
        snprintf(err, err_len, "%s", what);
    }
}

/* "file:" keeps a colon in a file name from being read as a protocol. */
static char *file_url(const char *path) { return av_asprintf("file:%s", path); }

void nitfeh_init(int verbose) { av_log_set_level(verbose ? AV_LOG_INFO : AV_LOG_QUIET); }

static int open_input(const char *path, AVFormatContext **in, const AVCodec **codec, char *err,
                      int err_len) {
    char *url = file_url(path);
    int ret;
    if (!url)
        return AVERROR(ENOMEM);
    ret = avformat_open_input(in, url, NULL, NULL);
    av_free(url);
    if (ret < 0) {
        fail(err, err_len, "cannot open the file", ret);
        return NITFEH_ERROR;
    }
    ret = avformat_find_stream_info(*in, NULL);
    if (ret < 0) {
        fail(err, err_len, "cannot read the file", ret);
        return NITFEH_ERROR;
    }
    ret = av_find_best_stream(*in, AVMEDIA_TYPE_AUDIO, -1, -1, codec, 0);
    if (ret == AVERROR_STREAM_NOT_FOUND) {
        fail(err, err_len, "no audio track", 0);
        return NITFEH_NO_AUDIO;
    }
    if (ret < 0) {
        fail(err, err_len, "unsupported audio format", 0);
        return NITFEH_ERROR;
    }
    return ret;
}

static int64_t stream_duration_us(const AVFormatContext *in, int stream) {
    const AVStream *st = in->streams[stream];
    if (st->duration != AV_NOPTS_VALUE && st->duration > 0)
        return av_rescale_q(st->duration, st->time_base, AV_TIME_BASE_Q);
    if (in->duration != AV_NOPTS_VALUE && in->duration > 0)
        return in->duration;
    return 0;
}

int nitfeh_probe(const char *path, NitfehInfo *info, char *err, int err_len) {
    AVFormatContext *in = NULL;
    const AVCodec *codec = NULL;
    int stream;

    memset(info, 0, sizeof(*info));
    stream = open_input(path, &in, &codec, err, err_len);
    if (stream >= 0) {
        const AVCodecParameters *par = in->streams[stream]->codecpar;
        info->has_audio = 1;
        info->duration_us = stream_duration_us(in, stream);
        info->sample_rate = par->sample_rate;
        info->channels = par->ch_layout.nb_channels;
        snprintf(info->codec, sizeof(info->codec), "%s", avcodec_get_name(par->codec_id));
    }
    avformat_close_input(&in);
    return stream >= 0 ? NITFEH_OK : stream;
}

static int open_decoder(Job *job, const AVCodec *codec) {
    AVStream *st = job->in->streams[job->stream];
    int ret;
    job->dec = avcodec_alloc_context3(codec);
    if (!job->dec)
        return AVERROR(ENOMEM);
    ret = avcodec_parameters_to_context(job->dec, st->codecpar);
    if (ret < 0)
        return ret;
    job->dec->pkt_timebase = st->time_base;
    return avcodec_open2(job->dec, codec, NULL);
}

static int open_output(Job *job, const char *path, const NitfehOptions *options) {
    const AVCodec *codec = avcodec_find_encoder_by_name("libopus");
    const AVDictionaryEntry *tag = NULL;
    const AVStream *in_stream = job->in->streams[job->stream];
    char *url;
    int ret;

    if (!codec)
        return AVERROR_ENCODER_NOT_FOUND;
    /* Named explicitly: the file is written under a temporary name without the .opus ending. */
    ret = avformat_alloc_output_context2(&job->out, NULL, "opus", NULL);
    if (ret < 0)
        return ret;

    job->enc = avcodec_alloc_context3(codec);
    if (!job->enc)
        return AVERROR(ENOMEM);
    job->enc->sample_rate = options->sample_rate;
    job->enc->sample_fmt = AV_SAMPLE_FMT_FLT;
    job->enc->bit_rate = options->bitrate;
    job->enc->time_base = (AVRational){1, options->sample_rate};
    av_channel_layout_default(&job->enc->ch_layout, 1);
    av_opt_set(job->enc->priv_data, "application", "voip", 0);
    av_opt_set_double(job->enc->priv_data, "frame_duration", options->frame_ms, 0);
    av_opt_set(job->enc->priv_data, "vbr", "on", 0);
    if (job->out->oformat->flags & AVFMT_GLOBALHEADER)
        job->enc->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
    ret = avcodec_open2(job->enc, codec, NULL);
    if (ret < 0)
        return ret;

    job->out_stream = avformat_new_stream(job->out, NULL);
    if (!job->out_stream)
        return AVERROR(ENOMEM);
    ret = avcodec_parameters_from_context(job->out_stream->codecpar, job->enc);
    if (ret < 0)
        return ret;
    job->out_stream->time_base = job->enc->time_base;

    av_dict_copy(&job->out->metadata, job->in->metadata, 0);
    while ((tag = av_dict_iterate(in_stream->metadata, tag)))
        if (!av_strcasecmp(tag->key, "language") || !av_strcasecmp(tag->key, "title"))
            av_dict_set(&job->out_stream->metadata, tag->key, tag->value, 0);

    job->fifo = av_audio_fifo_alloc(job->enc->sample_fmt, 1, job->enc->frame_size * 2);
    if (!job->fifo)
        return AVERROR(ENOMEM);

    url = file_url(path);
    if (!url)
        return AVERROR(ENOMEM);
    ret = avio_open(&job->out->pb, url, AVIO_FLAG_WRITE);
    av_free(url);
    if (ret < 0)
        return ret;
    return avformat_write_header(job->out, NULL);
}

static int write_packets(Job *job) {
    for (;;) {
        int ret = avcodec_receive_packet(job->enc, job->pkt);
        if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF)
            return 0;
        if (ret < 0)
            return ret;
        av_packet_rescale_ts(job->pkt, job->enc->time_base, job->out_stream->time_base);
        job->pkt->stream_index = job->out_stream->index;
        ret = av_interleaved_write_frame(job->out, job->pkt);
        if (ret < 0)
            return ret;
    }
}

/* Encodes whole frames from the fifo; with `flush`, also the short last frame and the encoder's tail. */
static int encode(Job *job, int flush) {
    const int frame_size = job->enc->frame_size;
    int ret;

    while (av_audio_fifo_size(job->fifo) >= frame_size ||
           (flush && av_audio_fifo_size(job->fifo) > 0)) {
        AVFrame *frame = job->frame;
        av_frame_unref(frame);
        frame->nb_samples = FFMIN(av_audio_fifo_size(job->fifo), frame_size);
        frame->format = job->enc->sample_fmt;
        frame->sample_rate = job->enc->sample_rate;
        ret = av_channel_layout_copy(&frame->ch_layout, &job->enc->ch_layout);
        if (ret < 0)
            return ret;
        ret = av_frame_get_buffer(frame, 0);
        if (ret < 0)
            return ret;
        if (av_audio_fifo_read(job->fifo, (void **)frame->data, frame->nb_samples) < frame->nb_samples)
            return AVERROR_BUG;
        frame->pts = job->samples;
        job->samples += frame->nb_samples;
        ret = avcodec_send_frame(job->enc, frame);
        av_frame_unref(frame);
        if (ret < 0)
            return ret;
        ret = write_packets(job);
        if (ret < 0)
            return ret;
    }
    if (flush) {
        ret = avcodec_send_frame(job->enc, NULL);
        if (ret < 0 && ret != AVERROR_EOF)
            return ret;
        return write_packets(job);
    }
    return 0;
}

static int buffer_for(Job *job, int samples) {
    if (samples <= job->buf_samples)
        return 0;
    av_freep(&job->buf);
    job->buf_samples = 0;
    if (av_samples_alloc(&job->buf, NULL, 1, samples, job->enc->sample_fmt, 0) < 0)
        return AVERROR(ENOMEM);
    job->buf_samples = samples;
    return 0;
}

/* Converts `in_samples` of input (or, with data == NULL, what the resampler still holds) into the fifo. */
static int convert(Job *job, const uint8_t *const *data, int in_samples) {
    int ret, max;
    if (!job->swr)
        return 0;
    max = swr_get_out_samples(job->swr, in_samples);
    if (max <= 0)
        return max;
    ret = buffer_for(job, max);
    if (ret < 0)
        return ret;
    ret = swr_convert(job->swr, &job->buf, max, data, in_samples);
    if (ret <= 0)
        return ret;
    return av_audio_fifo_write(job->fifo, (void **)&job->buf, ret) < ret ? AVERROR(ENOMEM) : 0;
}

static int resample(Job *job, const AVFrame *frame) {
    AVChannelLayout layout = {0};
    int ret;

    if (frame->ch_layout.order == AV_CHANNEL_ORDER_UNSPEC)
        av_channel_layout_default(&layout, frame->ch_layout.nb_channels);
    else if ((ret = av_channel_layout_copy(&layout, &frame->ch_layout)) < 0)
        return ret;

    if (!job->swr || frame->sample_rate != job->in_rate || frame->format != job->in_fmt ||
        av_channel_layout_compare(&layout, &job->in_layout)) {
        ret = convert(job, NULL, 0);
        if (ret < 0)
            goto end;
        swr_free(&job->swr);
        ret = swr_alloc_set_opts2(&job->swr, &job->enc->ch_layout, job->enc->sample_fmt,
                                  job->enc->sample_rate, &layout, frame->format, frame->sample_rate, 0,
                                  NULL);
        if (ret >= 0)
            ret = swr_init(job->swr);
        if (ret < 0) {
            swr_free(&job->swr);
            goto end;
        }
        job->in_rate = frame->sample_rate;
        job->in_fmt = frame->format;
        av_channel_layout_uninit(&job->in_layout);
        ret = av_channel_layout_copy(&job->in_layout, &layout);
        if (ret < 0)
            goto end;
    }
    ret = convert(job, (const uint8_t *const *)frame->extended_data, frame->nb_samples);
end:
    av_channel_layout_uninit(&layout);
    return ret;
}

/* Sends one packet (NULL to drain) to the decoder and moves every frame it yields to the encoder. */
static int decode(Job *job, const AVPacket *pkt, NitfehResult *result) {
    int ret = avcodec_send_packet(job->dec, pkt);
    if (ret < 0 && ret != AVERROR_EOF && ret != AVERROR(EAGAIN)) {
        result->decode_errors++;
        return 0;
    }
    for (;;) {
        ret = avcodec_receive_frame(job->dec, job->frame);
        if (ret == AVERROR(EAGAIN) || ret == AVERROR_EOF)
            return 0;
        if (ret < 0) {
            result->decode_errors++;
            return 0;
        }
        ret = resample(job, job->frame);
        av_frame_unref(job->frame);
        if (ret < 0)
            return ret;
        ret = encode(job, 0);
        if (ret < 0)
            return ret;
    }
}

static void close_job(Job *job) {
    av_freep(&job->buf);
    av_channel_layout_uninit(&job->in_layout);
    av_audio_fifo_free(job->fifo);
    swr_free(&job->swr);
    av_frame_free(&job->frame);
    av_packet_free(&job->pkt);
    avcodec_free_context(&job->dec);
    avcodec_free_context(&job->enc);
    if (job->out) {
        avio_closep(&job->out->pb);
        avformat_free_context(job->out);
    }
    avformat_close_input(&job->in);
}

int nitfeh_transcode(const char *in_path, const char *out_path, const NitfehOptions *options,
                     nitfeh_progress_fn progress, void *opaque, NitfehResult *result, char *err,
                     int err_len) {
    Job job = {0};
    const AVCodec *codec = NULL;
    int64_t total_us, reported_us = 0;
    int ret, status = NITFEH_ERROR;

    memset(result, 0, sizeof(*result));
    job.stream = open_input(in_path, &job.in, &codec, err, err_len);
    if (job.stream < 0) {
        status = job.stream;
        goto end;
    }
    for (unsigned i = 0; i < job.in->nb_streams; i++)
        if ((int)i != job.stream)
            job.in->streams[i]->discard = AVDISCARD_ALL;
    total_us = stream_duration_us(job.in, job.stream);

    job.pkt = av_packet_alloc();
    job.frame = av_frame_alloc();
    if (!job.pkt || !job.frame) {
        fail(err, err_len, "out of memory", 0);
        goto end;
    }
    if ((ret = open_decoder(&job, codec)) < 0) {
        fail(err, err_len, "cannot decode the audio", ret);
        goto end;
    }
    if ((ret = open_output(&job, out_path, options)) < 0) {
        fail(err, err_len, "cannot write the output", ret);
        goto end;
    }

    for (;;) {
        int64_t done_us;
        ret = av_read_frame(job.in, job.pkt);
        if (ret < 0) {
            result->truncated = ret != AVERROR_EOF;
            break;
        }
        ret = job.pkt->stream_index == job.stream ? decode(&job, job.pkt, result) : 0;
        av_packet_unref(job.pkt);
        if (ret < 0) {
            fail(err, err_len, "conversion failed", ret);
            goto end;
        }
        done_us = av_rescale(job.samples, AV_TIME_BASE, job.enc->sample_rate);
        if (progress && done_us - reported_us >= AV_TIME_BASE) {
            reported_us = done_us;
            if (progress(opaque, done_us, total_us)) {
                status = NITFEH_CANCELLED;
                goto end;
            }
        }
    }

    if ((ret = decode(&job, NULL, result)) < 0 || (ret = convert(&job, NULL, 0)) < 0 ||
        (ret = encode(&job, 1)) < 0 || (ret = av_write_trailer(job.out)) < 0) {
        fail(err, err_len, "conversion failed", ret);
        goto end;
    }
    if (job.out->pb->error < 0) {
        fail(err, err_len, "cannot write the output", job.out->pb->error);
        goto end;
    }
    if ((ret = avio_closep(&job.out->pb)) < 0) {
        fail(err, err_len, "cannot write the output", ret);
        goto end;
    }
    if (job.samples == 0) {
        fail(err, err_len, "no audio could be decoded", 0);
        goto end;
    }
    result->duration_us = av_rescale(job.samples, AV_TIME_BASE, job.enc->sample_rate);
    if (progress)
        progress(opaque, result->duration_us, result->duration_us);
    status = NITFEH_OK;

end:
    close_job(&job);
    return status;
}
