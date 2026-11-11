#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <stdlib.h>
#include <sys/file.h>
#include <sys/vfs.h>

int flock(int fd, int operation) {
    const char *error = getenv("DOWNLOAD_TEST_LOCK_ERROR");
    if (error) {
        errno = atoi(error);
        return -1;
    }
    if (getenv("DOWNLOAD_TEST_LOCAL_LOCKS")) {
        return 0;
    }
    int (*real_flock)(int, int) = dlsym(RTLD_NEXT, "flock");
    return real_flock(fd, operation);
}

int statfs(const char *path, struct statfs *buf) {
    const char *type = getenv("DOWNLOAD_TEST_FS_TYPE");
    if (type && strtol(type, NULL, 0) == -1) {
        errno = EIO;
        return -1;
    }
    int (*real_statfs)(const char *, struct statfs *) = dlsym(RTLD_NEXT, "statfs");
    int result = real_statfs(path, buf);
    if (result == 0 && type) {
        buf->f_type = strtol(type, NULL, 0);
    }
    return result;
}
