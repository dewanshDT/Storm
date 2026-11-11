#include "my_application.h"

#include <flutter_linux/flutter_linux.h>
#include <string.h>

#include "flutter/generated_plugin_registrant.h"
#include "storm_clipboard.h"

struct _MyApplication {
  GtkApplication parent_instance;
  char** dart_entrypoint_arguments;
};

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

// Called when first Flutter frame received.
static void first_frame_cb(MyApplication* self, FlView* view) {
  gtk_widget_show(gtk_widget_get_toplevel(GTK_WIDGET(view)));
}

// The icon a bundle carries next to its data, for window managers that read
// the window's own icon (X11, Alt-Tab). Wayland takes it from the .desktop.
static void set_window_icon(GtkWindow* window) {
  g_autofree gchar* exe = g_file_read_link("/proc/self/exe", nullptr);
  if (exe == nullptr) {
    return;
  }
  g_autofree gchar* dir = g_path_get_dirname(exe);
  g_autofree gchar* icon =
      g_build_filename(dir, "data", "storm_icon.png", nullptr);
  g_autoptr(GError) error = nullptr;
  if (!gtk_window_set_icon_from_file(window, icon, &error)) {
    g_warning("Window icon %s: %s", icon, error->message);
  }
}

// The storm/window channel: decorationLayout answers with the desktop's
// gtk-decoration-layout, e.g. "appmenu:minimize,maximize,close", so the
// caption buttons Storm draws follow its side and order.
static void window_method_cb(FlMethodChannel* channel, FlMethodCall* call,
                             gpointer user_data) {
  g_autoptr(FlMethodResponse) response = nullptr;
  if (strcmp(fl_method_call_get_name(call), "decorationLayout") == 0) {
    g_autofree gchar* layout = nullptr;
    g_object_get(gtk_settings_get_default(), "gtk-decoration-layout", &layout,
                 nullptr);
    g_autoptr(FlValue) result = fl_value_new_string(layout ? layout : "");
    response = FL_METHOD_RESPONSE(fl_method_success_response_new(result));
  } else {
    response = FL_METHOD_RESPONSE(fl_method_not_implemented_response_new());
  }
  g_autoptr(GError) error = nullptr;
  if (!fl_method_call_respond(call, response, &error)) {
    g_warning("storm/window: %s", error->message);
  }
}

static void attach_window_channel(FlView* view) {
  FlEngine* engine = fl_view_get_engine(view);
  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  FlMethodChannel* channel = fl_method_channel_new(
      fl_engine_get_binary_messenger(engine), "storm/window",
      FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(channel, window_method_cb, nullptr,
                                            nullptr);
  g_object_set_data_full(G_OBJECT(view), "storm-window", channel,
                         g_object_unref);
}

// Implements GApplication::activate.
static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);

  // A second launch arrives here, in this process, as another activate
  // (decision 125b). It raises the window that exists rather than starting a
  // second engine on the same drift cache and outbox.
  GList* windows = gtk_application_get_windows(GTK_APPLICATION(application));
  if (windows != nullptr) {
    gtk_window_present(GTK_WINDOW(windows->data));
    return;
  }

  GtkWindow* window =
      GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));

  // Storm draws its own title bar (decision 153d). An empty, unshown
  // titlebar widget keeps GTK's client-side frame, so the window still has
  // its shadow and resizes from its edges, with no header bar drawn.
  gtk_window_set_titlebar(window, gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0));
  gtk_window_set_title(window, "Storm");
  set_window_icon(window);

  gtk_window_set_default_size(window, 1280, 800);
  // The phone layout is the floor; below it nothing is designed for.
  GdkGeometry min_size = {};
  min_size.min_width = 360;
  min_size.min_height = 600;
  gtk_window_set_geometry_hints(window, nullptr, &min_size, GDK_HINT_MIN_SIZE);

  g_autoptr(FlDartProject) project = fl_dart_project_new();
  fl_dart_project_set_dart_entrypoint_arguments(
      project, self->dart_entrypoint_arguments);

  FlView* view = fl_view_new(project);
  GdkRGBA background_color;
  // Background defaults to black, override it here if necessary, e.g. #00000000
  // for transparent.
  gdk_rgba_parse(&background_color, "#000000");
  fl_view_set_background_color(view, &background_color);
  gtk_widget_show(GTK_WIDGET(view));
  gtk_container_add(GTK_CONTAINER(window), GTK_WIDGET(view));

  // Show the window when Flutter renders.
  // Requires the view to be realized so we can start rendering.
  g_signal_connect_swapped(view, "first-frame", G_CALLBACK(first_frame_cb),
                           self);
  gtk_widget_realize(GTK_WIDGET(view));

  fl_register_plugins(FL_PLUGIN_REGISTRY(view));
  storm_clipboard_attach(view);
  attach_window_channel(view);

  gtk_widget_grab_focus(GTK_WIDGET(view));
}

// Implements GApplication::local_command_line.
static gboolean my_application_local_command_line(GApplication* application,
                                                  gchar*** arguments,
                                                  int* exit_status) {
  MyApplication* self = MY_APPLICATION(application);
  // Strip out the first argument as it is the binary name.
  self->dart_entrypoint_arguments = g_strdupv(*arguments + 1);

  g_autoptr(GError) error = nullptr;
  if (!g_application_register(application, nullptr, &error)) {
    g_warning("Failed to register: %s", error->message);
    *exit_status = 1;
    return TRUE;
  }

  g_application_activate(application);
  *exit_status = 0;

  return TRUE;
}

// Implements GApplication::startup.
static void my_application_startup(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application startup.

  G_APPLICATION_CLASS(my_application_parent_class)->startup(application);
}

// Implements GApplication::shutdown.
static void my_application_shutdown(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application shutdown.

  G_APPLICATION_CLASS(my_application_parent_class)->shutdown(application);
}

// Implements GObject::dispose.
static void my_application_dispose(GObject* object) {
  MyApplication* self = MY_APPLICATION(object);
  g_clear_pointer(&self->dart_entrypoint_arguments, g_strfreev);
  G_OBJECT_CLASS(my_application_parent_class)->dispose(object);
}

static void my_application_class_init(MyApplicationClass* klass) {
  G_APPLICATION_CLASS(klass)->activate = my_application_activate;
  G_APPLICATION_CLASS(klass)->local_command_line =
      my_application_local_command_line;
  G_APPLICATION_CLASS(klass)->startup = my_application_startup;
  G_APPLICATION_CLASS(klass)->shutdown = my_application_shutdown;
  G_OBJECT_CLASS(klass)->dispose = my_application_dispose;
}

static void my_application_init(MyApplication* self) {}

MyApplication* my_application_new() {
  // Set the program name to the application ID, which helps various systems
  // like GTK and desktop environments map this running application to its
  // corresponding .desktop file. This ensures better integration by allowing
  // the application to be recognized beyond its binary name.
  g_set_prgname(APPLICATION_ID);

  // Unique, unlike the template's NON_UNIQUE (decision 125b). DEFAULT_FLAGS
  // arrived in GLib 2.74; the release builds on 22.04, which has 2.72.
#if GLIB_CHECK_VERSION(2, 74, 0)
  const GApplicationFlags flags = G_APPLICATION_DEFAULT_FLAGS;
#else
  const GApplicationFlags flags = G_APPLICATION_FLAGS_NONE;
#endif
  return MY_APPLICATION(g_object_new(my_application_get_type(),
                                     "application-id", APPLICATION_ID, "flags",
                                     flags, nullptr));
}
