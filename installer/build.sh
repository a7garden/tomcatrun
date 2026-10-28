#!/usr/bin/env bash
# tomcatrun 설치 파일을 만든다 — 실행 환경 받기 → 도우미 컴파일 → 릴리스 빌드 → stage/ 채우기 → makensis
# 결과: installer/dist/tomcatrun-setup-<버전>.exe
# 이 PC 에서도 GitHub Actions(.github/workflows/release.yml)에서도 같은 스크립트로 만든다.
#
# stage 에 넣는 것은 저장소의 파일과 공개된 곳에서 받은 실행 환경뿐이다.
# 넣지 않는 것: tomcatrun.json (처음 뜰 때 빈 서버 하나로 만든다) · 의존 jar(deps — 프로젝트마다 「의존 jar 다시 받기」로 받는다) ·
# 컴파일 결과 · 톰캣 logs · work · temp · web-dev.xml · app/ 소스 · 디자인 문서
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
stage="$here/stage"
version="$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$root/app/Cargo.toml" | head -1)"

makensis="${MAKENSIS:-}"
if [ -z "$makensis" ]; then
  for m in "$(command -v makensis || true)" "$LOCALAPPDATA/tauri/NSIS/makensis.exe" "/c/Program Files (x86)/NSIS/makensis.exe"; do
    if [ -n "$m" ] && [ -f "$m" ]; then makensis="$m"; break; fi
  done
fi
if [ -z "$makensis" ]; then
  echo "makensis 를 찾지 못했습니다 (NSIS 를 설치하거나 MAKENSIS 에 경로를 주세요)." >&2
  exit 1
fi

bash "$here/fetch.sh"
bash "$root/helper/build.sh"
(cd "$root/app" && cargo build --release)

rm -rf "$stage"
mkdir -p "$stage/helper" "$stage/base" "$here/dist"

cp "$root/app/target/release/tomcatrun.exe" "$root/README.md" "$stage/"
cp "$root/helper/"*.class "$stage/helper/"
cp -r "$root/overlay" "$stage/"
cp -r "$root/bin" "$stage/"
rm -rf "$stage/bin/dl"   # 내려받은 원본 묶음 · 빌드용 jar — 풀어 둔 것이 bin 에 이미 있다
cp -r "$root/base/conf" "$stage/base/"
rm -rf "$stage/base/conf/web-dev.xml" "$stage/base/conf/Catalina"

# 설치 파일에 이 PC 의 경로 · 프로젝트 이름이 섞이면 여기서 멈춘다 (설정 파일 · 톰캣 conf 는 모두 저장소의 것이어야 한다)
left="$(grep -rlI -e '[A-Za-z]:/workspace' -e '[A-Za-z]:\\workspace' "$stage/base" "$stage/overlay" || true)"
if [ -n "$left" ]; then
  echo "설치 파일에 이 PC 의 경로가 남았다: $left" >&2
  exit 1
fi

(cd "$here" && "$makensis" -V2 -DVERSION="$version" tomcatrun.nsi)
ls -la "$here/dist"
