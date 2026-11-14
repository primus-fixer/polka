#!/usr/bin/env bash
# Донастройка Android-проекта, который создаёт `tauri android init`:
# русское название, иконки, состояние сети и запрет облачной копии зашифрованной базы.
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

# INTERNET уже есть в шаблоне, CAMERA добавляет модуль сканера штрихкодов.
add_permission() {
  local name="$1"
  grep -q "android:name=\"$name\"" "$MANIFEST" && return 0
  sed -i "s#<application#<uses-permission android:name=\"$name\" />\n    <application#" "$MANIFEST"
}
add_permission android.permission.ACCESS_NETWORK_STATE

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

GRADLE="$PROJECT/app/build.gradle.kts"
[ -f "$GRADLE" ] && sed -i -E 's/targetSdk = [0-9]+/targetSdk = 36/' "$GRADLE"

echo "Android-проект донастроен:"
grep -E "uses-permission|allowBackup|dataExtractionRules|targetSdk|minSdk" -r "$MANIFEST" "$GRADLE" 2>/dev/null || true
