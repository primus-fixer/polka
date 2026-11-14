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
    echo "import android.content.Intent"
    echo "import android.os.Bundle"
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
    webView.addJavascriptInterface(ShareBridge(), "PolkaShare")
  }

  private fun remember(intent: Intent?) {
    if (intent == null || intent.action != Intent.ACTION_SEND) return
    val parts = listOfNotNull(intent.getStringExtra(Intent.EXTRA_SUBJECT), intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString())
    val text = parts.map { it.trim() }.filter { it.isNotEmpty() }.distinct().joinToString("\n")
    if (text.isNotEmpty()) shared.set(text.take(4000))
  }

  inner class ShareBridge {
    @JavascriptInterface
    fun take(): String? = shared.getAndSet(null)

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
  }
}
KT
  } > "$ACTIVITY.new"
  mv "$ACTIVITY.new" "$ACTIVITY"
fi

GRADLE="$PROJECT/app/build.gradle.kts"
[ -f "$GRADLE" ] && sed -i -E 's/targetSdk = [0-9]+/targetSdk = 36/' "$GRADLE"

echo "Android-проект донастроен:"
grep -E "uses-permission|allowBackup|dataExtractionRules|targetSdk|minSdk|intent.action" -r "$MANIFEST" "$GRADLE" 2>/dev/null || true
if [ -n "${ACTIVITY:-}" ]; then cat "$ACTIVITY"; fi
