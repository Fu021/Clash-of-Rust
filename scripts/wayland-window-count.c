#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wayland-client.h>

#include "ext-foreign-toplevel-list-v1-client.h"
#include "cosmic-toplevel-info-unstable-v1-client.h"
#include "cosmic-toplevel-management-unstable-v1-client.h"

struct window {
    struct wl_list link;
    struct ext_foreign_toplevel_handle_v1 *handle;
    struct zcosmic_toplevel_handle_v1 *cosmic;
    const char *expected_title;
    int32_t width;
    int32_t height;
    bool pending_match;
    bool matches;
};

struct probe {
    struct wl_list windows;
    struct ext_foreign_toplevel_list_v1 *list;
    struct zcosmic_toplevel_info_v1 *info;
    struct zcosmic_toplevel_manager_v1 *manager;
    const char *title;
    bool can_close;
    bool failed;
};

static void remove_window(struct window *window)
{
    if (window->cosmic)
        zcosmic_toplevel_handle_v1_destroy(window->cosmic);
    ext_foreign_toplevel_handle_v1_destroy(window->handle);
    wl_list_remove(&window->link);
    free(window);
}

static void geometry(void *data, struct zcosmic_toplevel_handle_v1 *handle,
                     struct wl_output *output, int32_t x, int32_t y,
                     int32_t width, int32_t height)
{
    (void) handle;
    (void) output;
    (void) x;
    (void) y;
    struct window *window = data;
    window->width = width;
    window->height = height;
}

static void ignore_output(void *data, struct zcosmic_toplevel_handle_v1 *handle,
                          struct wl_output *output)
{
    (void) data;
    (void) handle;
    (void) output;
}

static void ignore_state(void *data, struct zcosmic_toplevel_handle_v1 *handle,
                         struct wl_array *state)
{
    (void) data;
    (void) handle;
    (void) state;
}

static const struct zcosmic_toplevel_handle_v1_listener cosmic_listener = {
    .geometry = geometry,
    .output_enter = ignore_output,
    .output_leave = ignore_output,
    .state = ignore_state,
};

static void closed(void *data, struct ext_foreign_toplevel_handle_v1 *handle)
{
    (void) handle;
    remove_window(data);
}

static void done(void *data, struct ext_foreign_toplevel_handle_v1 *handle)
{
    (void) handle;
    struct window *window = data;
    window->matches = window->pending_match;
}

static void title(void *data, struct ext_foreign_toplevel_handle_v1 *handle,
                  const char *title)
{
    (void) handle;
    struct window *window = data;
    window->pending_match = strcmp(window->expected_title, title) == 0;
}

static void ignore_string(void *data, struct ext_foreign_toplevel_handle_v1 *handle,
                          const char *value)
{
    (void) data;
    (void) handle;
    (void) value;
}

static const struct ext_foreign_toplevel_handle_v1_listener handle_listener = {
    .closed = closed,
    .done = done,
    .title = title,
    .app_id = ignore_string,
    .identifier = ignore_string,
};

static void toplevel(void *data, struct ext_foreign_toplevel_list_v1 *list,
                     struct ext_foreign_toplevel_handle_v1 *handle)
{
    (void) list;
    struct probe *probe = data;
    struct window *window = calloc(1, sizeof(*window));
    if (!window) {
        probe->failed = true;
        ext_foreign_toplevel_handle_v1_destroy(handle);
        return;
    }
    window->handle = handle;
    window->expected_title = probe->title;
    wl_list_insert(&probe->windows, &window->link);
    window->cosmic = zcosmic_toplevel_info_v1_get_cosmic_toplevel(probe->info, handle);
    if (!window->cosmic
        || zcosmic_toplevel_handle_v1_add_listener(window->cosmic, &cosmic_listener, window) < 0
        || ext_foreign_toplevel_handle_v1_add_listener(handle, &handle_listener, window) < 0) {
        probe->failed = true;
        remove_window(window);
    }
}

static void finished(void *data, struct ext_foreign_toplevel_list_v1 *list)
{
    (void) list;
    struct probe *probe = data;
    probe->failed = true;
}

static const struct ext_foreign_toplevel_list_v1_listener list_listener = {
    .toplevel = toplevel,
    .finished = finished,
};

static void ignore_info_done(void *data, struct zcosmic_toplevel_info_v1 *info)
{
    (void) data;
    (void) info;
}

static const struct zcosmic_toplevel_info_v1_listener info_listener = {
    .done = ignore_info_done,
};

static void capabilities(void *data, struct zcosmic_toplevel_manager_v1 *manager,
                         struct wl_array *capabilities)
{
    (void) manager;
    struct probe *probe = data;
    probe->can_close = false;
    uint32_t *capability;
    wl_array_for_each(capability, capabilities) {
        if (*capability == ZCOSMIC_TOPLEVEL_MANAGER_V1_ZCOSMIC_TOPLELEVEL_MANAGEMENT_CAPABILITIES_V1_CLOSE)
            probe->can_close = true;
    }
}

static const struct zcosmic_toplevel_manager_v1_listener manager_listener = {
    .capabilities = capabilities,
};

static void global(void *data, struct wl_registry *registry, uint32_t name,
                   const char *interface, uint32_t version)
{
    struct probe *probe = data;
    if (strcmp(interface, ext_foreign_toplevel_list_v1_interface.name) == 0 && !probe->list) {
        probe->list = wl_registry_bind(registry, name, &ext_foreign_toplevel_list_v1_interface, 1);
        if (!probe->list || ext_foreign_toplevel_list_v1_add_listener(probe->list, &list_listener, probe) < 0)
            probe->failed = true;
    } else if (strcmp(interface, zcosmic_toplevel_info_v1_interface.name) == 0 && version >= 2 && !probe->info) {
        probe->info = wl_registry_bind(registry, name, &zcosmic_toplevel_info_v1_interface, 2);
        if (!probe->info || zcosmic_toplevel_info_v1_add_listener(probe->info, &info_listener, probe) < 0)
            probe->failed = true;
    } else if (strcmp(interface, zcosmic_toplevel_manager_v1_interface.name) == 0 && !probe->manager) {
        probe->manager = wl_registry_bind(registry, name, &zcosmic_toplevel_manager_v1_interface, 1);
        if (!probe->manager || zcosmic_toplevel_manager_v1_add_listener(probe->manager, &manager_listener, probe) < 0)
            probe->failed = true;
    } else if (strcmp(interface, wl_output_interface.name) == 0) {
        if (!wl_registry_bind(registry, name, &wl_output_interface, 1))
            probe->failed = true;
    }
}

static void global_remove(void *data, struct wl_registry *registry, uint32_t name)
{
    (void) data;
    (void) registry;
    (void) name;
}

static const struct wl_registry_listener registry_listener = {
    .global = global,
    .global_remove = global_remove,
};

int main(int argc, char **argv)
{
    if (argc != 3 || (strcmp(argv[1], "inspect") != 0 && strcmp(argv[1], "close") != 0)) {
        fprintf(stderr, "Usage: %s inspect|close WINDOW_TITLE\n", argv[0]);
        return EXIT_FAILURE;
    }
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) {
        fputs("Could not connect to the Wayland session\n", stderr);
        return EXIT_FAILURE;
    }
    struct probe probe = {.title = argv[2]};
    wl_list_init(&probe.windows);
    struct wl_registry *registry = wl_display_get_registry(display);
    bool ready = registry && wl_registry_add_listener(registry, &registry_listener, &probe) == 0
        && wl_display_roundtrip(display) >= 0 && probe.list && probe.info
        && wl_display_roundtrip(display) >= 0 && wl_display_roundtrip(display) >= 0
        && !probe.failed;
    unsigned int count = 0;
    struct window *matched = NULL;
    struct window *window, *next;
    wl_list_for_each(window, &probe.windows, link) {
        if (window->matches) {
            matched = window;
            count++;
        }
    }
    if (ready && strcmp(argv[1], "close") == 0) {
        if (count != 1 || !probe.can_close) {
            fprintf(stderr, "COSMIC close requires one matching toplevel and compositor support "
                            "(matches: %u, close advertised: %s)\n",
                    count, probe.can_close ? "yes" : "no");
            ready = false;
        } else {
            zcosmic_toplevel_manager_v1_close(probe.manager, matched->cosmic);
            ready = wl_display_roundtrip(display) >= 0;
        }
    } else if (ready) {
        putchar('[');
        const char *separator = "";
        wl_list_for_each(window, &probe.windows, link) {
            if (window->matches) {
                printf("%s{\"width\":%d,\"height\":%d}", separator, window->width, window->height);
                separator = ",";
            }
        }
        puts("]");
    }
    wl_list_for_each_safe(window, next, &probe.windows, link) {
        remove_window(window);
    }
    if (probe.list)
        ext_foreign_toplevel_list_v1_destroy(probe.list);
    if (probe.info)
        zcosmic_toplevel_info_v1_destroy(probe.info);
    if (probe.manager)
        zcosmic_toplevel_manager_v1_destroy(probe.manager);
    if (registry)
        wl_registry_destroy(registry);
    wl_display_disconnect(display);
    if (!ready) {
        fputs("The COSMIC window probe could not complete\n", stderr);
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
