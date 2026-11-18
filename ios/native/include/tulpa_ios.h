// C interface of the Tulpa runtime for Tulpa Field on iOS (implemented in ios/native/src/lib.rs).
//
// Strings passed in are borrowed NUL-terminated UTF-8 (NULL reads as ""). Strings returned are
// owned by the caller: release them with tf_string_free. Float buffers are released with
// tf_floats_free. The input callback must return a malloc'ed string (strdup) or NULL.
#ifndef TULPA_IOS_H
#define TULPA_IOS_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef void (*tf_output_cb)(void *ctx, const char *text);
typedef char *(*tf_input_cb)(void *ctx, const char *prompt);
typedef void (*tf_action_cb)(const char *kind, const char *payload);

void tf_init(tf_action_cb action);
void tf_string_free(char *s);
void tf_floats_free(float *p, int32_t len);
char *tf_version(void);
void tf_set_env(const char *name, const char *value);

int64_t tf_cancel_new(void);
void tf_cancel(int64_t h);
void tf_cancel_free(int64_t h);

// Returns NULL on success, otherwise the error message.
char *tf_run(const char *source, const char *file, const char *base_dir,
             const char *const *argv, int32_t argc, bool virtual_time,
             void *ctx, tf_output_cb out, tf_input_cb input, int64_t cancel);
char *tf_tests(const char *source, const char *file, const char *base_dir,
               void *ctx, tf_output_cb out, tf_input_cb input, int64_t cancel);
char *tf_verify(const char *source, const char *base_dir, int32_t trials, int64_t cancel);

int64_t tf_session_new(void *ctx, tf_output_cb out, tf_input_cb input, const char *base_dir);
char *tf_session_feed(int64_t h, const char *line);
void tf_session_interrupt(int64_t h);
void tf_session_cancel_entry(int64_t h);
char *tf_session_names(int64_t h);
void tf_session_free(int64_t h);

char *tf_diagnostics(const char *src);
char *tf_completion(const char *src, int32_t line, int32_t col);
char *tf_hover(const char *src, int32_t line, int32_t col);  // may return NULL
char *tf_symbols(const char *src);
char *tf_definition(const char *src, int32_t line, int32_t col);
char *tf_machines(const char *src);
char *tf_bytecode(const char *src);
char *tf_modules(void);

void tf_push_sensor(int32_t kind, const float *values, int32_t n);
void tf_push_location(double lat, double lon, double alt, double accuracy, double speed,
                      double bearing);
void tf_reset_sensors(void);

char *tf_sim_load(const char *path);
void tf_sim_free(int64_t h);
char *tf_sim_info(int64_t h);  // may return NULL
float *tf_sim_frame(int64_t h, int32_t *len);
float *tf_sim_mesh(int64_t h, int32_t id, int32_t *len);
char *tf_sim_serve(int64_t h, bool lan);
char *tf_sim_control(int64_t h, const char *cmd, double arg);
char *tf_sim_version(void);

#ifdef __cplusplus
}
#endif

#endif
