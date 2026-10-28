#!/usr/bin/env bash
# 자바 도우미 둘을 컴파일한다 — installer/fetch.sh 가 받은 JDK 8 · spring jar 로
#   helper/TomcatrunCompiler.class      상주 컴파일러
#   overlay/tomcatrun/DevTweaks*.class  개발 전용 앱 설정 (Spring BeanFactoryPostProcessor)
# 결과 .class 는 git 에 넣지 않는다. 소스를 고치면 이것을 다시 돌린다.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
javac="${JAVAC:-$root/bin/jdk8-dcevm/bin/javac}"
# javac 는 Windows 프로그램이라 「;」 로 이은 클래스패스를 Git Bash 가 바꿔 주지 않는다 — Windows 꼴 경로로 넘긴다
lib="$(cd "$root/bin/dl/build" && pwd -W)"

"$javac" -encoding UTF-8 -source 1.8 -target 1.8 -nowarn -d "$root/helper" "$root/helper/TomcatrunCompiler.java"
"$javac" -encoding UTF-8 -source 1.8 -target 1.8 -nowarn \
  -cp "$lib/spring-beans.jar;$lib/spring-core.jar" -d "$root/overlay" "$root/helper/DevTweaks.java"
echo "도우미 컴파일됨"
