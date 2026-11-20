#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int rename(const char *source, const char *target) {
    const char *name = strrchr(target, '/');
    const char *error = getenv("MINE_UTILS_TEST_OPTIONS_RENAME");
    if (error && name && strcmp(name + 1, "options.txt") == 0 && access(target, F_OK) == 0) {
        errno = atoi(error);
        return -1;
    }
    int (*real_rename)(const char *, const char *) = dlsym(RTLD_NEXT, "rename");
    return real_rename(source, target);
}
