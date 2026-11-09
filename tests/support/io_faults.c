/* Test-only syscall qualification. No production hooks. Build as a preload
 * library, or with IO_FAULTS_PROBE as a small independent ABI/scope probe.
 * Trace lives outside the canonical .avc scope. No stdio writes in hooks.
 * This is process failure testing, not a model of power loss or mmap stores. */
#define _GNU_SOURCE
#include <sys/types.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <limits.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef IO_FAULTS_PROBE
#include <assert.h>
#ifdef __APPLE__
static void probe_fcntl(int fd) {
    assert(fcntl(fd, F_GETFD) >= 0); /* no argument */
    assert(fcntl(fd, F_SETFD, FD_CLOEXEC) == 0); /* promoted int */
    char resolved[PATH_MAX];
    assert(fcntl(fd, F_GETPATH, resolved) == 0); /* char pointer */
    struct flock lock = {.l_type = F_WRLCK, .l_whence = SEEK_SET};
    assert(fcntl(fd, F_SETLK, &lock) == 0); /* struct pointer */
    assert(fcntl(fd, F_FULLFSYNC) == 0);
}
#endif
int main(int argc, char **argv) {
    assert(argc == 3);
    const char *kind = argv[1], *root = argv[2];
    char path[PATH_MAX];
    if (!strcmp(kind, "partial") || !strcmp(kind, "partial-pwrite")) {
        int positioned = !strcmp(kind, "partial-pwrite");
        snprintf(path, sizeof(path), "%s/torn", root);
        int fd = open(path, O_WRONLY); /* Not a creation event. */
        assert(fd >= 0);
        char data[] = "0123456789";
        assert((positioned ? pwrite(fd, data, 10, 7) : write(fd, data, 10)) == 5);
        assert(lseek(fd, 0, SEEK_CUR) == (positioned ? 0 : 5));
        errno = 0;
        assert((positioned ? pwrite(fd, data + 5, 5, 12) :
                write(fd, data + 5, 5)) == -1 && errno == EIO);
        assert(lseek(fd, 0, SEEK_CUR) == (positioned ? 0 : 5));
        assert(close(fd) == 0);
        return 0;
    }
    assert(!strcmp(kind, "scope"));
    /* These writes must NEVER enter the trace, even with fail-at=1. */
    const char *outside[] = {"../outside", "../.avc-sibling/file", "escape/file"};
    int dir = open(root, O_RDONLY | O_DIRECTORY);
    assert(dir >= 0);
    for (size_t i = 0; i < sizeof(outside) / sizeof(outside[0]); ++i) {
        int fd = openat(dir, outside[i], O_RDWR);
        assert(fd >= 0);
        assert(write(fd, "safe", 4) == 4);
        assert(pwrite(fd, "!", 1, 4) == 1);
        assert(fsync(fd) == 0);
#ifdef __APPLE__
        probe_fcntl(fd);
#endif
        assert(close(fd) == 0);
    }
    /* A dangling symlink must not classify its future outside target as scoped. */
    int escaped = openat(dir, "dangling", O_CREAT | O_WRONLY, 0600);
    assert(escaped >= 0 && write(escaped, "safe", 4) == 4);
    assert(close(escaped) == 0);
    /* dirfd-relative creation and directory sync. */
    int fd = openat(dir, "probe", O_CREAT | O_EXCL | O_RDWR, 0600);
    assert(fd >= 0);
    assert(write(fd, "abcdefghij", 10) == 10);
    assert(pwrite(fd, "XY", 2, 3) == 2);
    assert(lseek(fd, 0, SEEK_CUR) == 10);
    assert(fsync(fd) == 0);
#ifdef __APPLE__
    probe_fcntl(fd);
#endif
    assert(mkdirat(dir, "directory", 0700) == 0);
    assert(ftruncate(fd, 8) == 0);
    assert(fsync(dir) == 0);
    assert(close(fd) == 0 && close(dir) == 0);
    return 0;
}
#else
#include <dlfcn.h>
#include <stdatomic.h>

#ifdef __APPLE__
#define HOOK(name) fault_##name
#define NEXT(name) __typeof__(&name) next = &name
#define INTERPOSE(name) \
    __attribute__((used, section("__DATA,__interpose"))) \
    static const struct { __typeof__(&name) replacement, original; } ip_##name = \
        { &HOOK(name), &name }
#else
#define HOOK(name) name
#define NEXT(name) \
    __typeof__(&name) next = (__typeof__(&name))dlsym(RTLD_NEXT, #name); \
    if (!next) _exit(97)
#define INTERPOSE(name)
#endif

static char prefix[PATH_MAX];
static size_t prefix_len;
static int trace_fd = -1;
static unsigned long fail_at;
static const char *mode;
static _Atomic unsigned long ordinal;
static _Thread_local int logging;
/* The CLI is single-threaded. Pending short writes are per-thread and checked
 * against inode identity; close cancels the pending error before fd reuse. */
static _Thread_local int pending_fd = -1;
static _Thread_local struct stat pending_identity;

static int inside(const char *path) {
    return prefix_len && !strncmp(path, prefix, prefix_len) &&
           (path[prefix_len] == '/' || path[prefix_len] == '\0');
}

static int fd_path(int fd, char out[PATH_MAX]) {
    char raw[PATH_MAX];
#ifdef __APPLE__
    /* Forward through the same original binding as the fcntl hook (zero-vararg
     * misuse or self-recursion would abort in the ABI probe). */
    NEXT(fcntl);
    if (next(fd, F_GETPATH, raw) == -1) return 0;
#else
    char proc[64];
    snprintf(proc, sizeof(proc), "/proc/self/fd/%d", fd);
    ssize_t n = readlink(proc, raw, sizeof(raw) - 1);
    if (n < 0 || (size_t)n >= sizeof(raw) - 1) return 0;
    raw[n] = '\0';
#endif
    return realpath(raw, out) != NULL;
}

/* Resolve the actual existing target, or the canonical parent of a new leaf.
 * In particular, never treat a dangling symlink as a new scoped leaf. */
static int at_path(int dir, const char *path, char out[PATH_MAX]) {
    char joined[PATH_MAX], parent[PATH_MAX], base[PATH_MAX];
    if (path[0] == '/') {
        if (strlen(path) >= sizeof(joined)) return 0;
        strcpy(joined, path);
    } else {
        if (dir == AT_FDCWD) {
            if (!getcwd(base, sizeof(base))) return 0;
        } else if (!fd_path(dir, base)) return 0;
        int n = snprintf(joined, sizeof(joined), "%s/%s", base, path);
        if (n < 0 || (size_t)n >= sizeof(joined)) return 0;
    }
    if (realpath(joined, out)) return inside(out);
    struct stat st;
    if (lstat(joined, &st) == 0 || errno != ENOENT) return 0;
    char *leaf = strrchr(joined, '/');
    if (!leaf || !leaf[1]) return 0;
    *leaf++ = '\0';
    if (!realpath(joined, parent)) return 0;
    int n = snprintf(out, PATH_MAX, "%s/%s", parent, leaf);
    return n >= 0 && n < PATH_MAX && inside(out);
}

__attribute__((constructor)) static void setup(void) {
    int saved = errno;
    const char *root = getenv("AVC_IO_SCOPE");
    const char *trace = getenv("AVC_IO_TRACE");
    mode = getenv("AVC_IO_MODE");
    const char *nth = getenv("AVC_IO_NTH");
    if (!root || !trace || !mode || !nth || !realpath(root, prefix) ||
        strcmp(root, prefix)) _exit(97);
    prefix_len = strlen(prefix);
    if (prefix_len < 5 || strcmp(prefix + prefix_len - 5, "/.avc")) _exit(97);
    char resolved[PATH_MAX];
    if (!realpath(trace, resolved) || inside(resolved)) _exit(97);
    fail_at = strtoul(nth, NULL, 10);
    NEXT(open);
    trace_fd = next(resolved, O_WRONLY | O_APPEND | O_CLOEXEC);
    if (trace_fd < 0) _exit(97);
    errno = saved;
}

static void record(unsigned long n, const char *op, const char *path,
                   size_t bytes, off_t offset, const char *action) {
    char line[PATH_MAX + 160];
    int len = snprintf(line, sizeof(line), "%lu\t%s\t%s\t%zu\t%lld\t%s\n",
                       n, op, path + prefix_len, bytes, (long long)offset, action);
    if (len < 0 || (size_t)len >= sizeof(line)) _exit(97);
    NEXT(write);
    /* One append; no hook recursion, buffering, or log loss on _exit. */
    if (next(trace_fd, line, (size_t)len) != len) _exit(97);
}

static size_t short_count(size_t bytes) {
    return bytes / 2 < 137 ? bytes / 2 : 137;
}

/* Return 0=pass, 1=error, 2=short write. Preserve errno on every pass.
 * Trace describes attempted boundaries, not whether their real syscall won. */
static int event(const char *op, const char *path, size_t bytes, off_t offset,
                 int fd, int writable) {
    if (trace_fd < 0 || logging || !inside(path)) return 0;
    int saved = errno;
    logging = 1;
    unsigned long n = atomic_fetch_add(&ordinal, 1) + 1;
    int tail = 0;
    if (writable && fd == pending_fd) {
        struct stat st;
        tail = fstat(fd, &st) == 0 && st.st_dev == pending_identity.st_dev &&
               st.st_ino == pending_identity.st_ino;
        pending_fd = -1;
    }
    int hit = n == fail_at;
    int partial = hit && !strncmp(mode, "partial-", 8) && writable && bytes > 1;
    char partial_action[32];
    if (partial) snprintf(partial_action, sizeof(partial_action), "partial:%zu", short_count(bytes));
    const char *action = tail ? "tail-error" : !hit ? "pass" :
        partial ? partial_action : !strcmp(mode, "exit") ? "exit" : "error";
    record(n, op, path, bytes, offset, action);
    logging = 0;
    if (hit && !strcmp(mode, "exit")) _exit(86);
    if (partial) {
        if (fstat(fd, &pending_identity)) _exit(97);
        pending_fd = fd;
        errno = saved;
        return 2;
    }
    if (tail || hit) {
        errno = strstr(mode, "enospc") ? ENOSPC : EIO;
        return 1;
    }
    errno = saved;
    return 0;
}

static int fd_event(const char *op, int fd, size_t bytes, off_t offset, int write) {
    if (trace_fd < 0) return 0;
    int saved = errno;
    char path[PATH_MAX];
    int scoped = fd_path(fd, path) && inside(path);
    errno = saved;
    return scoped ? event(op, path, bytes, offset, fd, write) : 0;
}
static int path_event(const char *op, int dir, const char *path) {
    if (trace_fd < 0) return 0;
    int saved = errno;
    char canonical[PATH_MAX];
    int scoped = at_path(dir, path, canonical);
    errno = saved;
    return scoped ? event(op, canonical, 0, -1, -1, 0) : 0;
}

ssize_t HOOK(write)(int fd, const void *buf, size_t len) {
    NEXT(write);
    int saved = errno;
    off_t offset = lseek(fd, 0, SEEK_CUR);
    errno = saved;
    int fault = fd_event("write", fd, len, offset, 1);
    if (fault == 1) return -1;
    size_t count = fault == 2 ? short_count(len) : len;
    ssize_t result = next(fd, buf, count);
    if (fault == 2) {
        if (result != (ssize_t)count) _exit(97);
        if (!strcmp(mode, "partial-exit")) _exit(86);
    }
    return result;
}
INTERPOSE(write);

#define PWRITE(name) \
ssize_t HOOK(name)(int fd, const void *buf, size_t len, off_t offset) { \
    NEXT(name); \
    int fault = fd_event(#name, fd, len, offset, 1); \
    if (fault == 1) return -1; \
    size_t count = fault == 2 ? short_count(len) : len; \
    ssize_t result = next(fd, buf, count, offset); \
    if (fault == 2) { \
        if (result != (ssize_t)count) _exit(97); \
        if (!strcmp(mode, "partial-exit")) _exit(86); \
    } \
    return result; \
} \
INTERPOSE(name)
PWRITE(pwrite);
#ifndef __APPLE__
_Static_assert(sizeof(off_t) == sizeof(off64_t), "qualification requires 64-bit offsets");
PWRITE(pwrite64);
#endif

#define SYNC(name) \
int HOOK(name)(int fd) { \
    NEXT(name); \
    if (fd_event(#name, fd, 0, -1, 0)) return -1; \
    return next(fd); \
} \
INTERPOSE(name)
SYNC(fsync);
#ifndef __APPLE__
SYNC(fdatasync);
#endif

int HOOK(close)(int fd) {
    NEXT(close);
    /* dyld can call close before this image's TLS is initialized. */
    if (trace_fd >= 0 && fd == pending_fd) pending_fd = -1;
    return next(fd);
}
INTERPOSE(close);

static int creation(int flags) {
    return (flags & (O_CREAT | O_TRUNC)) != 0;
}
static int needs_mode(int flags) {
#ifdef O_TMPFILE
    if ((flags & O_TMPFILE) == O_TMPFILE) return 1;
#endif
    return (flags & O_CREAT) != 0;
}
#define OPEN(name) \
int HOOK(name)(const char *path, int flags, ...) { \
    NEXT(name); \
    mode_t perm = 0; \
    if (needs_mode(flags)) { \
        va_list ap; va_start(ap, flags); perm = va_arg(ap, int); va_end(ap); \
    } \
    if (creation(flags) && path_event(#name, AT_FDCWD, path)) return -1; \
    return needs_mode(flags) ? next(path, flags, perm) : next(path, flags); \
} \
INTERPOSE(name)
#define OPENAT(name) \
int HOOK(name)(int dir, const char *path, int flags, ...) { \
    NEXT(name); \
    mode_t perm = 0; \
    if (needs_mode(flags)) { \
        va_list ap; va_start(ap, flags); perm = va_arg(ap, int); va_end(ap); \
    } \
    if (creation(flags) && path_event(#name, dir, path)) return -1; \
    return needs_mode(flags) ? next(dir, path, flags, perm) : next(dir, path, flags); \
} \
INTERPOSE(name)
OPEN(open);
OPENAT(openat);
#ifndef __APPLE__
OPEN(open64);
OPENAT(openat64);
#endif

int HOOK(mkdir)(const char *path, mode_t perm) {
    NEXT(mkdir);
    if (path_event("mkdir", AT_FDCWD, path)) return -1;
    return next(path, perm);
}
INTERPOSE(mkdir);
int HOOK(mkdirat)(int dir, const char *path, mode_t perm) {
    NEXT(mkdirat);
    if (path_event("mkdirat", dir, path)) return -1;
    return next(dir, path, perm);
}
INTERPOSE(mkdirat);
int HOOK(ftruncate)(int fd, off_t length) {
    NEXT(ftruncate);
    if (fd_event("ftruncate", fd, (size_t)length, 0, 0)) return -1;
    return next(fd, length);
}
INTERPOSE(ftruncate);
#ifndef __APPLE__
int ftruncate64(int fd, off64_t length) {
    NEXT(ftruncate64);
    if (fd_event("ftruncate64", fd, (size_t)length, 0, 0)) return -1;
    return next(fd, length);
}
#endif

#ifdef __APPLE__
/* Darwin fcntl is variadic: do NOT read a nonexistent third argument for
 * F_FULLFSYNC, or read an int as a pointer. These are the CLI/libc/SQLite
 * forwarding families we qualify. Unknown commands fail the test (97) rather
 * than guessing a vararg type. No fcntl interposition is needed on Linux. */
int HOOK(fcntl)(int fd, int cmd, ...) {
    NEXT(fcntl);
    switch (cmd) {
    case F_FULLFSYNC: case F_BARRIERFSYNC:
        if (fd_event(cmd == F_FULLFSYNC ? "F_FULLFSYNC" : "F_BARRIERFSYNC",
                     fd, 0, -1, 0)) return -1;
        return next(fd, cmd);
    case F_GETFD: case F_GETFL: case F_GETOWN: case F_GETNOSIGPIPE:
        return next(fd, cmd);
    case F_DUPFD: case F_DUPFD_CLOEXEC: case F_SETFD: case F_SETFL:
    case F_SETOWN: case F_NOCACHE: case F_RDAHEAD: case F_SETNOSIGPIPE: {
        va_list ap; va_start(ap, cmd); int arg = va_arg(ap, int); va_end(ap);
        return next(fd, cmd, arg);
    }
    case F_GETPATH: {
        va_list ap; va_start(ap, cmd); char *arg = va_arg(ap, char *); va_end(ap);
        return next(fd, cmd, arg);
    }
    case F_GETLK: case F_SETLK: case F_SETLKW: {
        va_list ap; va_start(ap, cmd);
        struct flock *arg = va_arg(ap, struct flock *); va_end(ap);
        return next(fd, cmd, arg);
    }
    default: _exit(97);
    }
}
INTERPOSE(fcntl);
#endif
#endif
