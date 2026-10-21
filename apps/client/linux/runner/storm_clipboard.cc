#include "storm_clipboard.h"

#include <gtk/gtk.h>

#include <cstring>

namespace {

// Kept byte for byte, as the macOS runner does; anything else becomes a PNG.
bool is_kept_type(const gchar* path) {
  g_autofree gchar* lower = g_ascii_strdown(path, -1);
  static const char* const kept[] = {".png", ".jpg", ".jpeg", ".gif", ".webp"};
  for (const char* ext : kept) {
    if (g_str_has_suffix(lower, ext)) {
      return true;
    }
  }
  return false;
}

FlValue* png_of(GdkPixbuf* image) {
  g_autofree gchar* png = nullptr;
  gsize size = 0;
  if (!gdk_pixbuf_save_to_buffer(image, &png, &size, "png", nullptr,
                                 nullptr)) {
    return fl_value_new_null();
  }
  return fl_value_new_uint8_list(reinterpret_cast<const uint8_t*>(png), size);
}

// A file copied in a file manager arrives as a URI, not as image data.
FlValue* copied_file_image(GtkClipboard* clipboard) {
  g_auto(GStrv) uris = gtk_clipboard_wait_for_uris(clipboard);
  if (uris == nullptr || uris[0] == nullptr) {
    return nullptr;
  }
  g_autofree gchar* path = g_filename_from_uri(uris[0], nullptr, nullptr);
  if (path == nullptr) {
    return nullptr;
  }
  g_autoptr(GdkPixbuf) image = gdk_pixbuf_new_from_file(path, nullptr);
  if (image == nullptr) {
    return fl_value_new_null();
  }
  if (is_kept_type(path)) {
    g_autofree gchar* bytes = nullptr;
    gsize size = 0;
    if (g_file_get_contents(path, &bytes, &size, nullptr)) {
      return fl_value_new_uint8_list(reinterpret_cast<const uint8_t*>(bytes),
                                     size);
    }
  }
  return png_of(image);
}

FlValue* read_image() {
  GtkClipboard* clipboard = gtk_clipboard_get(GDK_SELECTION_CLIPBOARD);
  if (FlValue* file = copied_file_image(clipboard)) {
    return file;
  }
  g_autoptr(GdkPixbuf) image = gtk_clipboard_wait_for_image(clipboard);
  if (image == nullptr) {
    return fl_value_new_null();
  }
  return png_of(image);
}

void on_call(FlMethodChannel* channel, FlMethodCall* call, gpointer data) {
  g_autoptr(FlMethodResponse) response = nullptr;
  if (strcmp(fl_method_call_get_name(call), "readImage") == 0) {
    g_autoptr(FlValue) image = read_image();
    response = FL_METHOD_RESPONSE(fl_method_success_response_new(image));
  } else {
    response = FL_METHOD_RESPONSE(fl_method_not_implemented_response_new());
  }
  g_autoptr(GError) error = nullptr;
  if (!fl_method_call_respond(call, response, &error)) {
    g_warning("storm/clipboard: %s", error->message);
  }
}

}  // namespace

void storm_clipboard_attach(FlView* view) {
  FlEngine* engine = fl_view_get_engine(view);
  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  FlMethodChannel* channel = fl_method_channel_new(
      fl_engine_get_binary_messenger(engine), "storm/clipboard",
      FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(channel, on_call, nullptr, nullptr);
  g_object_set_data_full(G_OBJECT(view), "storm-clipboard", channel,
                         g_object_unref);
}
