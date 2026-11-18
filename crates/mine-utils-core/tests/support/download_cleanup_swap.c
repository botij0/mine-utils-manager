#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static void swap_parent(const char *path) {
    const char *target = getenv("CLEANUP_SWAP_TARGET");
    const char *outside = getenv("CLEANUP_SWAP_OUTSIDE");
    const char *moved = getenv("CLEANUP_SWAP_MOVED");
    if (!target || !outside || !moved || !strstr(path, ".mine-utils-download-")) {
        return;
    }
    unsetenv("CLEANUP_SWAP_TARGET");
    if (rename(target, moved) || symlink(outside, target)) {
        _exit(90);
    }
}

int unlink(const char *path) {
    swap_parent(path);
    int (*real_unlink)(const char *) = dlsym(RTLD_NEXT, "unlink");
    return real_unlink(path);
}

int unlinkat(int fd, const char *path, int flags) {
    swap_parent(path);
    int (*real_unlinkat)(int, const char *, int) = dlsym(RTLD_NEXT, "unlinkat");
    return real_unlinkat(fd, path, flags);
}
