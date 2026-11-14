#!/usr/bin/env bash
# Донастройка Android-проекта, который создаёт `tauri android init`:
# русское название, иконки, разрешения сети, «Поделиться» → «Полка» и запрет облачной копии зашифрованной базы.
set -euo pipefail

PROJECT="${1:?путь к gen/android}"
ICONS="${2:-}"
MAIN="$PROJECT/app/src/main"
MANIFEST="$MAIN/AndroidManifest.xml"

# Название под иконкой.
for f in "$MAIN"/res/values/strings.xml; do
  [ -f "$f" ] || continue
  sed -i -E 's#(<string name="(app_name|main_activity_title)">)[^<]*(</string>)#\1Полка\3#' "$f"
done

# Иконки, которые сделал `tauri icon`.
if [ -n "$ICONS" ] && [ -d "$ICONS" ]; then
  cp -r "$ICONS"/. "$MAIN/res/"
fi

# INTERNET уже есть в шаблоне, CAMERA добавляет модуль сканера штрихкодов и QR-кодов.
add_permission() {
  local name="$1"
  grep -q "android:name=\"$name\"" "$MANIFEST" && return 0
  sed -i "s#<application#<uses-permission android:name=\"$name\" />\n    <application#" "$MANIFEST"
}
add_permission android.permission.ACCESS_NETWORK_STATE
# Обмен по Wi-Fi: состояние Wi-Fi, широковещательный поиск устройств в сети и доступ к локальной сети.
add_permission android.permission.ACCESS_WIFI_STATE
add_permission android.permission.CHANGE_WIFI_MULTICAST_STATE
add_permission android.permission.ACCESS_LOCAL_NETWORK

# Резервное копирование Android выключено: база зашифрована ключом из Keystore,
# который на другой телефон не переносится, — копия там всё равно не откроется.
if ! grep -q 'android:allowBackup' "$MANIFEST"; then
  sed -i 's#<application#<application\n        android:allowBackup="false"\n        android:dataExtractionRules="@xml/data_extraction_rules"#' "$MANIFEST"
fi
mkdir -p "$MAIN/res/xml"
cat > "$MAIN/res/xml/data_extraction_rules.xml" <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<data-extraction-rules>
    <cloud-backup>
        <exclude domain="root" path="." />
        <exclude domain="file" path="." />
        <exclude domain="database" path="." />
        <exclude domain="sharedpref" path="." />
        <exclude domain="external" path="." />
    </cloud-backup>
    <device-transfer>
        <exclude domain="root" path="." />
        <exclude domain="file" path="." />
        <exclude domain="database" path="." />
        <exclude domain="sharedpref" path="." />
        <exclude domain="external" path="." />
    </device-transfer>
</data-extraction-rules>
XML

# «Поделиться» → «Полка»: ссылка или текст из Литреса, Кинопоиска, браузера открывает добавление.
if ! grep -q 'android.intent.action.SEND' "$MANIFEST"; then
  awk '
    /<\/activity>/ && !done {
      print "            <intent-filter>"
      print "                <action android:name=\"android.intent.action.SEND\" />"
      print "                <category android:name=\"android.intent.category.DEFAULT\" />"
      print "                <data android:mimeType=\"text/plain\" />"
      print "            </intent-filter>"
      done = 1
    }
    { print }
  ' "$MANIFEST" > "$MANIFEST.new"
  mv "$MANIFEST.new" "$MANIFEST"
fi

# Книги EPUB и FB2: «Открыть с помощью → Полка» из файлового менеджера, загрузок браузера, Telegram,
# и «Поделиться» файлом. FB2 Android не знает — такие файлы часто приходят как octet-stream или zip.
if ! grep -q 'application/epub+zip' "$MANIFEST"; then
  awk '
    function types() {
      print "                <data android:mimeType=\"application/epub+zip\" />"
      print "                <data android:mimeType=\"application/x-fictionbook+xml\" />"
      print "                <data android:mimeType=\"application/x-fictionbook\" />"
      print "                <data android:mimeType=\"application/x-fb2\" />"
      print "                <data android:mimeType=\"application/zip\" />"
      print "                <data android:mimeType=\"application/octet-stream\" />"
    }
    /<\/activity>/ && !done {
      print "            <intent-filter>"
      print "                <action android:name=\"android.intent.action.VIEW\" />"
      print "                <category android:name=\"android.intent.category.DEFAULT\" />"
      print "                <data android:scheme=\"content\" />"
      print "                <data android:scheme=\"file\" />"
      types()
      print "            </intent-filter>"
      print "            <intent-filter>"
      print "                <action android:name=\"android.intent.action.SEND\" />"
      print "                <category android:name=\"android.intent.category.DEFAULT\" />"
      types()
      print "            </intent-filter>"
      done = 1
    }
    { print }
  ' "$MANIFEST" > "$MANIFEST.new"
  mv "$MANIFEST.new" "$MANIFEST"
fi

# Картинка года уходит в «Поделиться» через FileProvider из шаблона Tauri; если его нет — добавляем.
if ! grep -q 'androidx.core.content.FileProvider' "$MANIFEST"; then
  awk '
    /<\/application>/ && !done {
      print "        <provider"
      print "            android:name=\"androidx.core.content.FileProvider\""
      print "            android:authorities=\"${applicationId}.fileprovider\""
      print "            android:exported=\"false\""
      print "            android:grantUriPermissions=\"true\">"
      print "            <meta-data"
      print "                android:name=\"android.support.FILE_PROVIDER_PATHS\""
      print "                android:resource=\"@xml/file_paths\" />"
      print "        </provider>"
      done = 1
    }
    { print }
  ' "$MANIFEST" > "$MANIFEST.new"
  mv "$MANIFEST.new" "$MANIFEST"
fi
mkdir -p "$MAIN/res/xml"
if [ ! -f "$MAIN/res/xml/file_paths.xml" ]; then
  cat > "$MAIN/res/xml/file_paths.xml" <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<paths>
    <cache-path name="share" path="share/" />
</paths>
XML
fi

# Текст из «Поделиться» ждёт в активности, пока страница заберёт его: window.PolkaShare.take().
# Страница в этом окне — только своя, другие сайты в нём не открываются.
ACTIVITY=$(find "$MAIN" -name MainActivity.kt | head -1)
if [ -n "$ACTIVITY" ] && ! grep -q PolkaShare "$ACTIVITY"; then
  PACKAGE=$(grep -m1 '^package ' "$ACTIVITY")
  EDGE=""
  grep -q enableEdgeToEdge "$ACTIVITY" && EDGE="    enableEdgeToEdge()"
  {
    echo "$PACKAGE"
    echo
    echo "import android.content.ContentValues"
    echo "import android.content.Intent"
    echo "import android.net.Uri"
    echo "import android.os.Build"
    echo "import android.os.Bundle"
    echo "import android.os.Environment"
    echo "import android.provider.MediaStore"
    echo "import android.provider.OpenableColumns"
    echo "import android.util.Base64"
    echo "import android.webkit.JavascriptInterface"
    echo "import android.webkit.WebView"
    [ -n "$EDGE" ] && echo "import androidx.activity.enableEdgeToEdge"
    echo "import androidx.core.content.FileProvider"
    echo "import java.io.File"
    echo "import java.util.concurrent.atomic.AtomicReference"
    cat <<'KT'

class MainActivity : TauriActivity() {
  /** Текст из «Поделиться», пока его не забрала страница. */
  private val shared = AtomicReference<String?>(null)
  /** Запись, которую открыли из виджета («» — просто «Полка»). */
  private val opened = AtomicReference<String?>(null)
  /** Файл книги в кеше приложения (имя), пока его не забрала страница. */
  private val book = AtomicReference<String?>(null)
  private var web: WebView? = null

  override fun onCreate(savedInstanceState: Bundle?) {
KT
    [ -n "$EDGE" ] && echo "$EDGE"
    cat <<'KT'
    super.onCreate(savedInstanceState)
    remember(intent)
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    remember(intent)
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    web = webView
    webView.addJavascriptInterface(ShareBridge(), "PolkaShare")
  }

  private fun remember(intent: Intent?) {
    if (intent == null) return
    if (intent.action == NowWidget.OPEN) {
      opened.set(intent.getStringExtra("id") ?: "")
      return
    }
    if (intent.action == Intent.ACTION_VIEW) {
      intent.data?.let { receiveBook(it) }
      return
    }
    if (intent.action != Intent.ACTION_SEND) return
    val stream = streamOf(intent)
    if (stream != null && intent.type?.startsWith("text/") != true) {
      receiveBook(stream)
      return
    }
    val parts = listOfNotNull(intent.getStringExtra(Intent.EXTRA_SUBJECT), intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString())
    val text = parts.map { it.trim() }.filter { it.isNotEmpty() }.distinct().joinToString("\n")
    if (text.isNotEmpty()) shared.set(text.take(4000))
  }

  @Suppress("DEPRECATION")
  private fun streamOf(intent: Intent): Uri? =
    if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
    else intent.getParcelableExtra(Intent.EXTRA_STREAM) as? Uri

  /** Книга — копией в кеш приложения (не больше 100 МБ); страница заберёт её: PolkaShare.takeBook(). */
  private fun receiveBook(uri: Uri) {
    val resolver = contentResolver
    Thread {
      var file: File? = null
      try {
        var name = uri.lastPathSegment ?: "book"
        if (uri.scheme == "content") {
          resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0)?.let { name = it }
          }
        }
        val safe = name.substringAfterLast('/').replace(Regex("[^\\p{L}\\p{N} ._-]"), "_").take(120).ifBlank { "book" }
        val dir = File(cacheDir, "incoming").apply { mkdirs() }
        dir.listFiles()?.filter { System.currentTimeMillis() - it.lastModified() > 86_400_000L }?.forEach { it.delete() }
        val target = File(dir, "${System.currentTimeMillis()}-$safe")
        file = target
        val input = resolver.openInputStream(uri) ?: return@Thread
        input.use { src ->
          target.outputStream().use { out ->
            val buf = ByteArray(64 * 1024)
            var total = 0L
            while (true) {
              val n = src.read(buf)
              if (n < 0) break
              total += n
              if (total > 100L * 1024 * 1024) throw IllegalStateException("слишком большой файл")
              out.write(buf, 0, n)
            }
          }
        }
        book.set(target.name)
        runOnUiThread { web?.evaluateJavascript("window.dispatchEvent(new Event('polka-book'))", null) }
      } catch (e: Exception) {
        file?.delete()
      }
    }.start()
  }

  inner class ShareBridge {
    @JavascriptInterface
    fun take(): String? = shared.getAndSet(null)

    /** Запись из виджета: номер, «» — просто открыть, null — ничего. */
    @JavascriptInterface
    fun takeOpen(): String? = opened.getAndSet(null)

    /** Книга, открытая «С помощью…» или присланная через «Поделиться»: имя файла в кеше. */
    @JavascriptInterface
    fun takeBook(): String? = book.getAndSet(null)

    /** Что показать в виджете «Сейчас»: JSON от страницы. */
    @JavascriptInterface
    fun setWidget(json: String) {
      NowWidget.save(applicationContext, json.take(512 * 1024))
    }

    /** Картинка года: файл в кэше приложения и окно «Поделиться» (сохранить, отправить). */
    @JavascriptInterface
    fun sendImage(base64: String, name: String): Boolean {
      return try {
        val bytes = Base64.decode(base64, Base64.DEFAULT)
        val dir = File(cacheDir, "share").apply { mkdirs() }
        dir.listFiles()?.forEach { it.delete() }
        val safe = name.replace(Regex("[^\\p{L}\\p{N} ._-]"), "_").take(80).ifBlank { "polka" }
        val file = File(dir, if (safe.endsWith(".png")) safe else "$safe.png")
        file.writeBytes(bytes)
        val uri = FileProvider.getUriForFile(this@MainActivity, "$packageName.fileprovider", file)
        val send = Intent(Intent.ACTION_SEND)
          .setType("image/png")
          .putExtra(Intent.EXTRA_STREAM, uri)
          .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        runOnUiThread { startActivity(Intent.createChooser(send, null)) }
        true
      } catch (e: Exception) {
        false
      }
    }

    /** Android 10 и новее: картинку можно положить в «Галерею» без разрешений. */
    @JavascriptInterface
    fun canSaveImage(): Boolean = Build.VERSION.SDK_INT >= 29

    /** Картинка — в «Изображения/Полка»; возвращает, куда легла, или null. */
    @JavascriptInterface
    fun saveImage(base64: String, name: String): String? {
      if (Build.VERSION.SDK_INT < 29) return null
      return try {
        val bytes = Base64.decode(base64, Base64.DEFAULT)
        val safe = name.replace(Regex("[^\\p{L}\\p{N} ._-]"), "_").take(80).ifBlank { "polka" }
        val file = if (safe.endsWith(".png")) safe else "$safe.png"
        val values = ContentValues().apply {
          put(MediaStore.Images.Media.DISPLAY_NAME, file)
          put(MediaStore.Images.Media.MIME_TYPE, "image/png")
          put(MediaStore.Images.Media.RELATIVE_PATH, Environment.DIRECTORY_PICTURES + "/Полка")
          put(MediaStore.Images.Media.IS_PENDING, 1)
        }
        val uri = contentResolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values) ?: return null
        val written = contentResolver.openOutputStream(uri)?.use { it.write(bytes); true } ?: false
        if (!written) {
          contentResolver.delete(uri, null, null)
          return null
        }
        values.clear()
        values.put(MediaStore.Images.Media.IS_PENDING, 0)
        contentResolver.update(uri, values, null, null)
        "Изображения/Полка"
      } catch (e: Exception) {
        null
      }
    }
  }
}
KT
  } > "$ACTIVITY.new"
  mv "$ACTIVITY.new" "$ACTIVITY"
fi

# Виджет «Сейчас читаю» на главный экран: что читаете и смотрите, нажатие открывает запись.
# Данные присылает страница «Полки» (window.PolkaShare.setWidget), пока приложение открыто.
if [ -n "${ACTIVITY:-}" ]; then
  WDIR=$(dirname "$ACTIVITY")
  PACKAGE=$(grep -m1 '^package ' "$ACTIVITY")
  {
    echo "$PACKAGE"
    cat <<'KT'

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Color
import android.net.Uri
import android.util.Base64
import android.view.View
import android.widget.RemoteViews
import org.json.JSONObject

/** Виджет «Сейчас»: что читаете и смотрите. Данные присылает «Полка», пока открыта. */
class NowWidget : AppWidgetProvider() {
  override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
    val views = render(context)
    for (id in ids) manager.updateAppWidget(id, views)
  }

  companion object {
    const val OPEN = "app.polka.OPEN"
    private const val PREFS = "polka_widget"
    private const val KEY = "now"

    private val ROWS = intArrayOf(R.id.polka_row1, R.id.polka_row2, R.id.polka_row3)
    private val COVERS = intArrayOf(R.id.polka_cover1, R.id.polka_cover2, R.id.polka_cover3)
    private val TITLES = intArrayOf(R.id.polka_title1, R.id.polka_title2, R.id.polka_title3)
    private val SUBS = intArrayOf(R.id.polka_sub1, R.id.polka_sub2, R.id.polka_sub3)
    private val BARS = intArrayOf(R.id.polka_bar1, R.id.polka_bar2, R.id.polka_bar3)

    fun save(context: Context, json: String) {
      context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(KEY, json).apply()
      val manager = AppWidgetManager.getInstance(context)
      val ids = manager.getAppWidgetIds(ComponentName(context, NowWidget::class.java))
      if (ids.isEmpty()) return
      val views = render(context)
      for (id in ids) manager.updateAppWidget(id, views)
    }

    private fun open(context: Context, code: Int, id: String?): PendingIntent {
      val intent = Intent(context, MainActivity::class.java).setAction(OPEN).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
      if (!id.isNullOrEmpty()) {
        intent.putExtra("id", id)
        intent.data = Uri.parse("polka://open/" + Uri.encode(id))
      }
      return PendingIntent.getActivity(context, code, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    }

    private fun placeholder(color: String): Bitmap {
      val b = Bitmap.createBitmap(48, 72, Bitmap.Config.ARGB_8888)
      b.eraseColor(try { Color.parseColor(color) } catch (e: Exception) { Color.GRAY })
      return b
    }

    private fun cover(data: String): Bitmap? {
      if (data.isEmpty()) return null
      return try {
        val bytes = Base64.decode(data, Base64.DEFAULT)
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
      } catch (e: Exception) {
        null
      }
    }

    fun render(context: Context): RemoteViews {
      val views = RemoteViews(context.packageName, R.layout.polka_widget)
      views.setOnClickPendingIntent(R.id.polka_widget_root, open(context, 99, null))
      val json = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY, null)
      val items = try { JSONObject(json ?: "{}").optJSONArray("items") } catch (e: Exception) { null }
      val count = minOf(items?.length() ?: 0, ROWS.size)
      views.setViewVisibility(R.id.polka_empty, if (count == 0) View.VISIBLE else View.GONE)
      for (i in ROWS.indices) {
        val item = if (items != null && i < count) items.optJSONObject(i) else null
        if (item == null) {
          views.setViewVisibility(ROWS[i], View.GONE)
          continue
        }
        views.setViewVisibility(ROWS[i], View.VISIBLE)
        views.setTextViewText(TITLES[i], item.optString("title"))
        views.setTextViewText(SUBS[i], item.optString("sub"))
        val pct = item.optInt("pct", -1)
        if (pct >= 0) {
          views.setViewVisibility(BARS[i], View.VISIBLE)
          views.setProgressBar(BARS[i], 100, pct.coerceIn(0, 100), false)
        } else {
          views.setViewVisibility(BARS[i], View.GONE)
        }
        views.setImageViewBitmap(COVERS[i], cover(item.optString("cover")) ?: placeholder(item.optString("bg", "#4A5560")))
        views.setOnClickPendingIntent(ROWS[i], open(context, 100 + i, item.optString("id")))
      }
      return views
    }
  }
}
KT
  } > "$WDIR/NowWidget.kt"

  mkdir -p "$MAIN/res/layout" "$MAIN/res/xml" "$MAIN/res/drawable" "$MAIN/res/values" "$MAIN/res/values-night"
  {
    cat <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<LinearLayout xmlns:android="http://schemas.android.com/apk/res/android"
    android:id="@+id/polka_widget_root"
    android:layout_width="match_parent"
    android:layout_height="match_parent"
    android:background="@drawable/polka_widget_bg"
    android:orientation="vertical"
    android:padding="14dp">

    <TextView
        android:layout_width="wrap_content"
        android:layout_height="wrap_content"
        android:letterSpacing="0.1"
        android:text="@string/polka_widget_title"
        android:textColor="@color/polka_widget_muted"
        android:textSize="11sp"
        android:textStyle="bold" />

    <TextView
        android:id="@+id/polka_empty"
        android:layout_width="match_parent"
        android:layout_height="wrap_content"
        android:paddingTop="10dp"
        android:text="@string/polka_widget_empty"
        android:textColor="@color/polka_widget_ink"
        android:textSize="13sp" />
XML
    for i in 1 2 3; do
      cat <<XML

    <LinearLayout
        android:id="@+id/polka_row$i"
        android:layout_width="match_parent"
        android:layout_height="wrap_content"
        android:gravity="center_vertical"
        android:orientation="horizontal"
        android:paddingTop="8dp"
        android:visibility="gone">

        <ImageView
            android:id="@+id/polka_cover$i"
            android:layout_width="32dp"
            android:layout_height="48dp"
            android:contentDescription="@null"
            android:scaleType="centerCrop" />

        <LinearLayout
            android:layout_width="0dp"
            android:layout_height="wrap_content"
            android:layout_weight="1"
            android:orientation="vertical"
            android:paddingStart="10dp">

            <TextView
                android:id="@+id/polka_title$i"
                android:layout_width="match_parent"
                android:layout_height="wrap_content"
                android:ellipsize="end"
                android:maxLines="1"
                android:textColor="@color/polka_widget_ink"
                android:textSize="14sp"
                android:textStyle="bold" />

            <TextView
                android:id="@+id/polka_sub$i"
                android:layout_width="match_parent"
                android:layout_height="wrap_content"
                android:ellipsize="end"
                android:maxLines="1"
                android:textColor="@color/polka_widget_muted"
                android:textSize="12sp" />

            <ProgressBar
                android:id="@+id/polka_bar$i"
                style="?android:attr/progressBarStyleHorizontal"
                android:layout_width="match_parent"
                android:layout_height="4dp"
                android:layout_marginTop="4dp"
                android:max="100"
                android:progressBackgroundTint="@color/polka_widget_track"
                android:progressTint="@color/polka_widget_bar"
                android:visibility="gone" />
        </LinearLayout>
    </LinearLayout>
XML
    done
    echo '</LinearLayout>'
  } > "$MAIN/res/layout/polka_widget.xml"

  cat > "$MAIN/res/xml/polka_widget_info.xml" <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<appwidget-provider xmlns:android="http://schemas.android.com/apk/res/android"
    android:description="@string/polka_widget_description"
    android:initialLayout="@layout/polka_widget"
    android:minHeight="110dp"
    android:minWidth="250dp"
    android:previewLayout="@layout/polka_widget"
    android:resizeMode="horizontal|vertical"
    android:targetCellHeight="2"
    android:targetCellWidth="4"
    android:updatePeriodMillis="0"
    android:widgetCategory="home_screen" />
XML

  cat > "$MAIN/res/drawable/polka_widget_bg.xml" <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<shape xmlns:android="http://schemas.android.com/apk/res/android" android:shape="rectangle">
    <solid android:color="@color/polka_widget_bg" />
    <corners android:radius="20dp" />
</shape>
XML

  cat > "$MAIN/res/values/polka_widget.xml" <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<resources>
    <string name="polka_widget_label">Полка — сейчас</string>
    <string name="polka_widget_title">СЕЙЧАС</string>
    <string name="polka_widget_empty">Сейчас ничего не читаете и не смотрите. Нажмите, чтобы открыть «Полку».</string>
    <string name="polka_widget_description">Что вы сейчас читаете и смотрите</string>
    <color name="polka_widget_bg">#F4F6F8</color>
    <color name="polka_widget_ink">#15232D</color>
    <color name="polka_widget_muted">#566671</color>
    <color name="polka_widget_track">#DCE3E8</color>
    <color name="polka_widget_bar">#1F5E8C</color>
</resources>
XML

  cat > "$MAIN/res/values-night/polka_widget.xml" <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<resources>
    <color name="polka_widget_bg">#1C2A33</color>
    <color name="polka_widget_ink">#EEF2F5</color>
    <color name="polka_widget_muted">#A9B8C2</color>
    <color name="polka_widget_track">#33434D</color>
    <color name="polka_widget_bar">#8CC2E8</color>
</resources>
XML

  if ! grep -q 'NowWidget' "$MANIFEST"; then
    awk '
      /<\/application>/ && !done {
        print "        <receiver"
        print "            android:name=\".NowWidget\""
        print "            android:exported=\"false\""
        print "            android:label=\"@string/polka_widget_label\">"
        print "            <intent-filter>"
        print "                <action android:name=\"android.appwidget.action.APPWIDGET_UPDATE\" />"
        print "            </intent-filter>"
        print "            <meta-data"
        print "                android:name=\"android.appwidget.provider\""
        print "                android:resource=\"@xml/polka_widget_info\" />"
        print "        </receiver>"
        done = 1
      }
      { print }
    ' "$MANIFEST" > "$MANIFEST.new"
    mv "$MANIFEST.new" "$MANIFEST"
  fi
fi

GRADLE="$PROJECT/app/build.gradle.kts"
[ -f "$GRADLE" ] && sed -i -E 's/targetSdk = [0-9]+/targetSdk = 36/' "$GRADLE"

echo "Android-проект донастроен:"
grep -E "uses-permission|allowBackup|dataExtractionRules|targetSdk|minSdk|intent.action" -r "$MANIFEST" "$GRADLE" 2>/dev/null || true
if [ -n "${ACTIVITY:-}" ]; then cat "$ACTIVITY"; ls "$(dirname "$ACTIVITY")"; fi
