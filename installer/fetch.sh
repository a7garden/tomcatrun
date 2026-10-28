#!/usr/bin/env bash
# 실행기가 함께 쓰는 실행 환경을 bin/ 에 받는다 — DCEVM JDK 8 · HotswapAgent · Maven
# 이미 있는 것은 건너뛴다. 받은 파일은 모두 해시를 대조한 뒤에만 푼다.
#
#   bin/jdk8-dcevm      AdoptOpenJDK 8u181-b13 + DCEVM 8u181 build2 (jre/bin/dcevm/jvm.dll · -XXaltjvm=dcevm 으로 고른다)
#   bin/hotswap-agent.jar   HotswapAgent 1.4.1
#   bin/maven           Apache Maven 3.9.16
#   bin/dl/build        도우미 컴파일에만 쓰는 spring-beans · spring-core 4.3.22 (설치 파일에 넣지 않는다)
#
# 버전을 바꾸면 해시도 함께 바꾼다. DCEVM 8u181 은 JDK 8u181 에만 맞는다 — JDK 판을 따로 올리지 않는다.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="${1:-$root/bin}"
dl="$bin/dl"
mkdir -p "$dl/build"

JDK_URL="https://github.com/AdoptOpenJDK/openjdk8-binaries/releases/download/jdk8u181-b13/OpenJDK8U-jdk_x64_windows_hotspot_8u181b13.zip"
JDK_SHA256="54970581fbbfeba55e1e394769e2ac4208fa8354fc4c1134be9560942e5ddefb"
DCEVM_URL="https://github.com/dcevm/dcevm/releases/download/light-jdk8u181%2B2/DCEVM-8u181-installer-build2.jar"
DCEVM_SHA256="5e955fb5923106098eba23b9176a92ff51402d5a5710aa79360a3e6e6cf27951"
HSA_URL="https://github.com/HotswapProjects/HotswapAgent/releases/download/RELEASE-1.4.1/hotswap-agent-1.4.1.jar"
HSA_SHA256="a496305b110f12837c1dfd59d20c02737ce2a06647ebfe02b77ffe13ee756872"
MVN_URL="https://archive.apache.org/dist/maven/maven-3/3.9.16/binaries/apache-maven-3.9.16-bin.zip"
MVN_SHA512="ed41650d42485cfc243fad22158caf9cbb5dc408ce7a09ddb94dd42a019de929ca43065bfa450612cf12bf78b5cafa3884b96c090de326ff590448c933454af3"
SPRING="https://repo1.maven.org/maven2/org/springframework"

# get <주소> <파일> <해시 명령> <해시>
# 해시는 표준 입력으로 잰다 — 파일 이름에 역슬래시가 있으면 sha256sum 이 출력 앞에 「\」 를 붙인다
get() {
  local url="$1" out="$2" sum="$3" want="$4"
  hash_of() { "$sum" < "$1" | cut -d' ' -f1; }
  if [ ! -f "$out" ] || [ "$(hash_of "$out")" != "$want" ]; then
    echo "받기: $url"
    curl -fsSL --retry 3 -o "$out.part" "$url"
    mv "$out.part" "$out"
  fi
  local got
  got="$(hash_of "$out")"
  if [ "$got" != "$want" ]; then
    echo "해시가 다릅니다: $out ($got)" >&2
    exit 1
  fi
}

if [ ! -f "$bin/jdk8-dcevm/jre/bin/dcevm/jvm.dll" ]; then
  get "$JDK_URL" "$dl/jdk8u181.zip" sha256sum "$JDK_SHA256"
  get "$DCEVM_URL" "$dl/dcevm-8u181-installer.jar" sha256sum "$DCEVM_SHA256"
  rm -rf "$bin/jdk8-dcevm" "$dl/jdk"
  unzip -q "$dl/jdk8u181.zip" -d "$dl/jdk"
  mv "$dl/jdk"/jdk8u181* "$bin/jdk8-dcevm"
  rm -rf "$dl/jdk"
  # 설치기를 띄우지 않고 그 안의 jvm.dll 만 꺼내 altjvm 자리에 둔다 — 설치기의 「Install as altjvm」 과 같은 결과다
  mkdir -p "$bin/jdk8-dcevm/jre/bin/dcevm"
  unzip -q -p "$dl/dcevm-8u181-installer.jar" windows_amd64_compiler2/product/jvm.dll > "$bin/jdk8-dcevm/jre/bin/dcevm/jvm.dll"
fi

if [ ! -f "$bin/hotswap-agent.jar" ]; then
  get "$HSA_URL" "$dl/hotswap-agent-1.4.1.jar" sha256sum "$HSA_SHA256"
  cp "$dl/hotswap-agent-1.4.1.jar" "$bin/hotswap-agent.jar"
fi

if [ ! -f "$bin/maven/bin/mvn.cmd" ]; then
  get "$MVN_URL" "$dl/maven.zip" sha512sum "$MVN_SHA512"
  rm -rf "$bin/maven" "$dl/mvn"
  unzip -q "$dl/maven.zip" -d "$dl/mvn"
  mv "$dl/mvn"/apache-maven-* "$bin/maven"
  rm -rf "$dl/mvn"
fi

get "$SPRING/spring-beans/4.3.22.RELEASE/spring-beans-4.3.22.RELEASE.jar" "$dl/build/spring-beans.jar" sha1sum b996bdca37e04a9e875dd40db184df208a315356
get "$SPRING/spring-core/4.3.22.RELEASE/spring-core-4.3.22.RELEASE.jar" "$dl/build/spring-core.jar" sha1sum 644c452cb43edf55f3cc8a1252a6e5a28316f42e

echo "실행 환경 준비됨: $bin"
