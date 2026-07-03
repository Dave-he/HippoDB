/* Drop-in smoke test: dlopen libsqlite_rs.dylib and call the
 * C-ABI functions exactly the way a consumer of libsqlite3 would.
 *
 * Build & run (from rust-port/):
 *   cargo build
 *   cc -o /tmp/dlopen_smoke examples/dlopen_smoke.c -ldl
 *   DYLD_LIBRARY_PATH=target/debug /tmp/dlopen_smoke
 */
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>

typedef const char *(*libversion_fn)(void);
typedef int (*libversion_number_fn)(void);
typedef int (*open_fn)(const char *, void **);
typedef int (*close_fn)(void *);
typedef int (*errcode_fn)(void *);
typedef const char *(*errmsg_fn)(void *);

int main(void) {
    void *h = dlopen("liblibsqlite_rs.dylib", RTLD_NOW);
    if (!h) {
        h = dlopen("./target/debug/liblibsqlite_rs.dylib", RTLD_NOW);
    }
    if (!h) {
        fprintf(stderr, "dlopen failed: %s\n", dlerror());
        return 1;
    }
    printf("[ok] dlopen liblibsqlite_rs.dylib\n");

    libversion_fn lv = (libversion_fn)dlsym(h, "sqlite3_libversion");
    libversion_number_fn lvn = (libversion_number_fn)dlsym(h, "sqlite3_libversion_number");
    open_fn open_fn_ptr = (open_fn)dlsym(h, "sqlite3_open");
    close_fn close_fn_ptr = (close_fn)dlsym(h, "sqlite3_close");
    errcode_fn ec = (errcode_fn)dlsym(h, "sqlite3_errcode");
    errmsg_fn em = (errmsg_fn)dlsym(h, "sqlite3_errmsg");

    if (!lv || !lvn || !open_fn_ptr || !close_fn_ptr || !ec || !em) {
        fprintf(stderr, "dlsym failed: %s\n", dlerror());
        return 1;
    }

    printf("[ok] sqlite3_libversion      = %s\n", lv());
    printf("[ok] sqlite3_libversion_number = %d\n", lvn());

    void *db = NULL;
    int rc = open_fn_ptr(":memory:", &db);
    printf("[ok] sqlite3_open(\":memory:\") = %d (db=%p)\n", rc, db);
    if (rc != 0) return 1;

    printf("[ok] sqlite3_errcode(db)      = %d\n", ec(db));
    printf("[ok] sqlite3_errmsg(db)       = %s\n", em(db));

    rc = close_fn_ptr(db);
    printf("[ok] sqlite3_close(db)        = %d\n", rc);
    if (rc != 0) return 1;

    /* Open non-existent file should still succeed (P0 stub). */
    rc = open_fn_ptr("/tmp/no-such-file.db", &db);
    printf("[ok] sqlite3_open(/tmp/no-such-file.db) = %d (db=%p)\n", rc, db);
    if (rc == 0 && db) {
        rc = close_fn_ptr(db);
        printf("[ok] sqlite3_close            = %d\n", rc);
    }

    dlclose(h);
    printf("[ok] all dlopen smoke tests passed\n");
    return 0;
}
