# tomcatrun

로컬 톰캣 개발 실행기 (Windows).

IDE 의 서버 배포 없이 Spring · 서블릿 웹앱을 로컬 톰캣으로 띄우고, 고친 것을 재시작 없이 반영한다.
프로젝트마다 톰캣 하나를 등록해 따로 켜고 끄며, 여럿을 함께 띄울 수 있다.

- JSP · JS · CSS 는 저장하면 바로, 자바는 저장하면 바뀐 파일만 컴파일해 실행 중인 JVM 에 교체한다 (DCEVM · HotswapAgent).
- 무엇이 바로 들어갔고 무엇이 재시작해야 들어가는지를 「반영 기록」과 알림 줄로 보여 준다.
- 프로젝트 소스 · IDE 설정 · 버전 관리 작업복사본은 읽기만 한다. 실행기가 쓰는 것은 모두 실행기 폴더 안이다.

## 설치

1. [릴리스](../../releases)에서 `tomcatrun-setup-<버전>.exe` 를 받아 실행한다. 기본 위치는 `%LOCALAPPDATA%\Programs\tomcatrun` 이고(이미 깔려 있으면 그 자리) 관리자 권한이 필요 없다.
2. **톰캣 8.5** 를 따로 설치한다 (기본 자리 `C:\Program Files\Apache Software Foundation\Tomcat 8.5`). 다른 자리면 「설정 › 경로」에서 정한다.

설치 파일에는 실행기 · DCEVM JDK 8 (AdoptOpenJDK 8u181 + DCEVM) · HotswapAgent · Maven 이 들어 있다. JDK 를 따로 깔지 않아도 된다.

## 시작하기

처음 켜면 「시작하기」 창이 뜬다.

- **이클립스를 쓰고 있었다면** — 작업 공간을 알아서 찾아 쓰던 톰캣 서버를 보여 준다. 「가져오기」 한 번이면 서버 이름 · 프로젝트 폴더 · 포트 · 톰캣 폴더 · 의존 jar(이클립스 배포본의 `WEB-INF/lib`)까지 들어온다. 바로 「시작」을 누르면 된다. 이클립스 설정은 읽기만 한다.
- **아니면** — 「프로젝트 폴더 고르기」(`src/main/webapp` 이 있는 폴더) 하나. 의존 jar 는 Maven 으로 받는다.

작업 공간을 못 찾으면 「이클립스 작업 공간 고르기」로 고른다. 나중에 다시 하려면 설정 › 서버 › 「이클립스에서 가져오기」.

## 설정 — 회사 · 팀마다 다른 값은 직접 정한다

실행기에는 특정 프로젝트의 경로 · 포트 · 접속 주소 · 저장소 주소가 들어 있지 않다. 「시작하기」가 채우지 못한 것은
「설정」(탭 줄 오른쪽 끝 · Ctrl+,)에서 고친다. 저장하면 실행기 폴더의 `tomcatrun.json` 에 남는다.

| 설정 › 칸 | 무엇 |
| - | - |
| 서버 › 이름 | 서버 줄 · 탭에 보이는 이름 (예: 내부망 · 외부망) |
| 서버 › 프로젝트 폴더 | `src/main/webapp` 이 있는 Maven 표준 폴더. 「고르기」로 저장소 맨 위를 골라도 바로 아래 한 단계에서 찾는다 |
| 서버 › HTTP · AJP · 종료 · 디버거 포트 | 서버마다 겹치지 않게. AJP 는 같은 PC 의 앞단(Apache mod_jk 등)용이라 `127.0.0.1` 에만 열린다 |
| 서버 › 바로가기 | 서버 줄 「바로가기」 메뉴. 「바로가기 추가」로 이름 · 주소를 한 줄씩 더한다. `/` 로 시작하는 주소(`/` · `/main.do`)는 그 서버의 HTTP 포트로 열린다 |
| 서버 › JVM 인자 | 그 서버에만 더할 인자. 회사 · 팀 전용 시스템 속성(`-D…`)은 여기 둔다 |
| 경로 › 톰캣 폴더 · javac · java · 편집기 | 기본은 설치된 JDK 8 · `code`(VS Code) |
| 경로 › TLD jar | 태그 라이브러리를 찾을 jar (쉼표로). 비우면 모든 jar 를 본다 |
| 버전 관리 | 서버마다 SVN · Git · 작업 폴더 · 커밋 메시지 예시 · 업데이트 명령 |
| 저장 감시 제외 | 저장해도 반영 기록 · 재시작 알림에 넣지 않을 파일 · 폴더 |

**실행기가 알아서 정하는 것은 없다.** 서버 이름 · 프로젝트 폴더 · 바로가기는 PC 마다 사람이 정한다 — 어느 폴더가 어느 망(내부망 · 외부망 등)의 소스인지 실행기는 감지하지 않는다.
포트만 기본값이 있다. n 번째 서버(0부터)는 HTTP `8080+n` · AJP `8009+n` · 종료 `8005+100n` · 디버거 `8000+n` 이고, 다른 서버나 지금 이 PC 에서 듣고 있는 프로그램(IDE 톰캣 등)과 겹치면 빈 자리까지 올린다. 새 서버의 바로가기는 `/` 하나다.

「서버 추가」로 서버를 더한다(실행기를 다시 열면 보인다). 더한 서버의 톰캣 설정 · 컴파일 결과 · 의존 jar 는 `servers/<id>/` 아래에 생긴다.
SVN 저장소 주소는 작업복사본(`.svn/wc.db`)에서 읽고, 로그인이 필요하면 처음 한 번 묻는다. 계정은 **Windows 자격 증명 관리자**(`tomcatrun:svn:<호스트>`)에만 두고 화면 · 로그 · 설정 파일에 남기지 않는다.

> `tomcatrun.json` 은 그 PC 의 설정이다. 회사 내부 경로 · 주소가 들어가므로 공개 저장소에 올리지 않는다 (이 저장소의 `.gitignore` 도 뺀다).

그다음 서버 줄의 「의존 jar 다시 받기」로 `pom.xml` 의 의존 jar 를 받고(사내 Maven 저장소가 필요하면 `%USERPROFILE%\.m2\settings.xml` 에 둔다), 「시작」을 누른다.

## 업데이트

켤 때와 6시간마다 [릴리스](../../releases)의 최신 판을 확인한다. 새 판이 있으면 알림 줄에 「업데이트」 단추가 뜬다.
누르면 설치 파일을 받아 SHA-256 을 릴리스와 대조하고, 켜진 톰캣을 정상 종료한 뒤 같은 폴더에 조용히 설치하고 실행기를 다시 연다.
설정 · 의존 jar · 컴파일 결과는 그대로 남는다. 「설정 › 업데이트」에서 자동 확인을 끄거나 지금 확인한다.

GitHub 연결은 Windows 인증서 저장소를 쓰고 `HTTPS_PROXY` 환경 변수를 따른다. 사내망에서 닿지 않으면 조용히 넘어간다(설정 › 업데이트에만 보인다).

## 무엇이 어떻게 반영되나

| 고친 것 | 반영 | 까닭 |
| - | - | - |
| JSP | 저장 후 1초 안 | 톰캣이 `src/main/webapp` 을 직접 읽는다 · JSP 확인 주기 1초 |
| JS · CSS | 새로고침하면 바로 | 브라우저가 매번 서버에 다시 확인한다 (`conf/web.xml` 의 DevNoCache) |
| 매퍼 XML (스스로 다시 읽는 묶음) | 저장 후 몇 초 | 앱의 `RefreshableSqlSessionFactoryBean` 이 원본을 보고 그 묶음을 다시 해석한다. 「매퍼 하나만 다시 읽기」(실험)를 켜면 바뀐 파일만 |
| 자바 | 저장 후 1초 안 | 바뀐 파일만 상주 ECJ 로 컴파일 → HotswapAgent 가 실행 중인 JVM 에 교체 |
| Spring 설정 · web.xml · 그 밖의 매퍼 · 메시지 · pom 의존성 | 재시작 | 기동 때 한 번만 읽는다 |

재시작이 필요한 것은 서버 줄 아래 노란 알림 줄에 서버별로 모이고 그 서버의 재시작 단추에 개수가 붙는다. HotswapAgent 가 교체에 실패해도 여기에 올라온다.
스스로 다시 읽는 매퍼 묶음은 프로젝트의 `src/main/resources/spring/context-datasource.xml` 에서 `RefreshableSqlSessionFactoryBean` 의 `mapperLocations` 를 읽어 정한다 (서버를 켤 때마다).

### 저장 감시 제외 패턴

| 패턴 | 뜻 |
| - | - |
| `a.sql` | 어느 폴더에서나 이 이름의 파일 (같은 이름의 폴더면 그 아래 전부) |
| `*.sql` | 어느 폴더에서나 이 확장자 |
| `src/main/resources/db/a.sql` | 그 파일 하나만 |
| `src/main/resources/db/deploy/` | 그 폴더 아래 전부 |
| `deploy/` | 어디에 있든 이 이름의 폴더 아래 전부 |
| `src/main/*/db` | `*` 는 한 단계 |
| `src/**/tmp/` | `**` 는 여러 단계 |

대소문자 · 역슬래시는 가리지 않는다. 탐색기에서 복사한 전체 경로를 넣으면 프로젝트 기준으로 바꿔 저장한다.

## 옵션 (「설정」)

| 옵션 | 기본 | 무엇 |
| - | - | - |
| 자바 핫리로드 | 켬 | DCEVM JDK 와 HotswapAgent 로 띄운다. 메서드 · 필드 추가까지 재시작 없이 들어간다 |
| 저장하면 컴파일 · 컴파일러 | 켬 · ECJ | 상주 컴파일러를 톰캣 lib 의 ECJ 로 (없으면 javac) |
| 디버거 연결 허용 | 끔 | 서버의 디버거 포트로 IDE 가 원격 디버그로 붙는다 |
| 톰캣 힙 상한 | 1.5GB | `-Xmx` + SerialGC · 남은 힙을 운영체제에 돌려준다 |
| 빠른 기동 JVM 옵션 | 켬 | `-XX:TieredStopAtLevel=1 -Xverify:none` |
| 클래스 전체 탐색 건너뛰기 | 끔 | web.xml 사본에 `metadata-complete="true"` 와 빈 `<absolute-ordering/>` 를 더하고 웹소켓 초기화를 거른다. 서블릿 3.0 애너테이션 · 초기화 클래스를 쓰지 않는 앱에서만 켠다 |
| 서버를 차례로 띄우기 | 켬 | 먼저 누른 서버가 다 뜬 뒤 다음 서버를 띄운다 |
| DB 연결 미리 열기 1개 | 켬 | commons-dbcp `BasicDataSource` 의 처음 연결 수를 1로 |
| 예약 작업 멈추기 | 끔 | `@Scheduled` 를 돌리지 않는다 |
| 지연 초기화 (실험) | 끔 | 빈을 처음 쓸 때 만든다 |
| 매퍼 하나만 다시 읽기 (실험) | 끔 | HotswapAgent MyBatis 플러그인 · 앱의 묶음 감시를 끈다 |
| 재시작해도 로그인 유지 | 켬 | 정상 종료 때 세션을 저장했다가 되살린다 (톰캣 기본 동작) |

**개발 전용 앱 설정** (`overlay/`): `tomcatrun.DevTweaks` 가 실행기가 주는 시스템 속성(`tomcatrun.*`)에 따라 Spring 빈 정의만 고친다. 속성이 없으면 아무것도 하지 않는다.
앱이 `classpath*:spring/context-*.xml` · `classpath*:egovframework/springmvc/*.xml` 로 설정을 읽는 구성(전자정부 표준프레임워크 꼴)에서 원본과 함께 읽힌다. 다른 구성이면 위 네 옵션(DB 연결 · 예약 작업 · 지연 초기화 · 매퍼 하나만)이 동작하지 않을 뿐 나머지는 그대로다.

**기동 단계 측정**: 기동이 끝나면 반영 기록에 단계별 시간이 남고, `logs/startup-history.csv` 에 기동마다 한 줄씩 쌓인다.

## 버전 관리 (SVN · Git 읽기 전용)

「버전 관리」 탭. 서버마다 SVN · Git · 둘 다 · 안 씀을 고른다 (기본은 작업 폴더의 `.svn/wc.db` · `.git` 을 보고 자동).

| 보기 | 무엇 |
| - | - |
| 내 변경 | 바뀐 파일 · 기준본과의 차이 · 저장소 최신본과 비교 · 이 파일의 로그 |
| 업데이트 확인 | 업데이트하면 들어올 커밋 · 그중 내가 고친 파일과 겹치는 것 |
| 로그 | 최근 커밋 · 바뀐 파일 · 커밋의 차이 |

**실행기는 저장소에 쓰지 않는다.** 커밋 · 업데이트 · 되돌리기 단추는 없고, 명령을 복사만 해 준다.
SVN 상태는 `.svn/wc.db` 를 읽기 전용(immutable)으로 열어 계산하고, 저장소 조회는 mod_dav_svn 에 읽기 요청(`OPTIONS` · `REPORT` · `GET`)만 보낸다. Git 은 저장소를 바꾸지 않는 명령만 부른다.

## 그 밖에

- 컴파일 오류 줄의 `파일:줄` 을 누르면 편집기가 그 줄에서 열린다.
- 우클릭하면 그 자리에 맞는 메뉴가 뜬다. 서버 칸은 시작 · 중지 · 바로가기 · 폴더, 파일이 걸린 줄(반영 기록 · 로그 · 재시작 목록 · 버전 관리 목록 · 차이 보기)은 편집기로 열기 · 탐색기에서 보기 · 경로 복사 · 저장 감시에서 빼기를 고른다. 브라우저 기본 메뉴(새로 고침 등)는 입력칸에서만 뜬다.
- 톰캣 콘솔 전체가 서버마다 `logs/console.log` 에 남는다. 직전 기동분은 `console.prev.log`.
- 켜는 중에 20초 넘게 멈춰 있던 톰캣을 중지하면 스레드 상태를 `logs/threads-<시각>.txt` 로 남긴다.
- 실행기가 비정상으로 끝나도 톰캣이 같이 끝난다 (Windows 작업 개체).
- 실행기는 하나만 뜬다. 다시 실행하면 떠 있는 창이 앞으로 온다.
- **IDE 와 같은 서버를 동시에 켜지 않는다.** 실행기는 기동 전에 포트를 확인하고, 막혀 있으면 잡고 있는 프로그램 이름과 PID 를 보여 준다.

## 한계

- 클래스 상속 구조를 바꾸거나 AOP 가 걸린 클래스의 메서드 시그니처를 크게 바꾸면 교체에 실패할 수 있다. 반영 기록에 오류가 나오면 재시작한다.
- static 초기화 블록은 다시 돌지 않는다. 이미 만들어진 객체에 새 필드 초깃값은 채워지지 않는다.
- 자바 파일을 지우면 그 클래스가 컴파일 폴더에 남는다. 톰캣을 끈 상태에서 「전체」 컴파일을 하면 비우고 다시 만든다.
- 톰캣 8.5 · 자바 8 앱을 대상으로 만들었다 (DCEVM 8u181).

## 폴더

| 폴더 | 무엇 |
| - | - |
| `base/conf/` | 톰캣 설정 한 벌. 경로 · 포트를 적지 않고 실행기가 기동 때 `tomcatrun.*` 속성으로 넘긴다. 첫 서버는 `base/` 를, 더한 서버는 `servers/<id>/base` 에 이것을 복사해 쓴다 |
| `classes/` · `deps/` | 첫 서버의 컴파일 결과 · 의존 jar |
| `bin/` | DCEVM JDK 8 · HotswapAgent · Maven (`installer/fetch.sh` 가 받는다) |
| `helper/` | 상주 컴파일러(`TomcatrunCompiler`) · 개발 전용 앱 설정(`DevTweaks`) 소스 · `build.sh` |
| `overlay/` | 개발 전용 앱 설정. 모든 서버의 `/WEB-INF/classes` 에 덧붙는다 |
| `app/` | 실행기 소스 (Rust · Tauri 2) |
| `installer/` | 설치 파일 (`fetch.sh` · `build.sh` · `tomcatrun.nsi`) |
| `tomcatrun.json` | 이 PC 의 설정 (git 에 넣지 않는다) |

## 소스에서 만들기

Windows · Git Bash · Rust(stable) · NSIS 가 필요하다.

```bash
bash installer/fetch.sh     # bin/ 에 JDK · HotswapAgent · Maven 받기 (해시 대조)
bash helper/build.sh        # 도우미 컴파일
cd app && cargo test --release && cargo build --release
bash installer/build.sh     # 위를 모두 하고 installer/dist/tomcatrun-setup-<버전>.exe
```

개발 중에는 저장소 폴더 자체가 실행기 폴더다 (`app/target/release/tomcatrun.exe` 가 위로 올라가며 `base/conf/server.xml` 을 찾는다).

## 릴리스

버전은 `app/Cargo.toml` 의 `version` 한 곳에서 정한다 (`CHANGELOG.md`). `v<버전>` 태그를 올리면 GitHub Actions(`.github/workflows/release.yml`)가 시험 · 빌드 후 설치 파일을 릴리스에 붙인다.

```bash
git tag v0.6.0 && git push origin v0.6.0
```

## 라이선스

tomcatrun 의 소스는 [MIT License](LICENSE) 다.

설치 파일에는 아래를 원래 배포본 그대로 넣는다. tomcatrun 은 이것들을 별도 프로세스로 띄울 뿐 한 프로그램으로 묶지 않으므로, 각자 자기 라이선스를 따른다.

| 무엇 | 라이선스 | 소스 |
| - | - | - |
| AdoptOpenJDK 8u181-b13 (`bin/jdk8-dcevm`) | GPLv2 + Classpath Exception | 설치 폴더의 `bin/jdk8-dcevm/src.zip` · [openjdk8-binaries](https://github.com/AdoptOpenJDK/openjdk8-binaries/releases/tag/jdk8u181-b13) |
| DCEVM light 8u181 build2 (`jre/bin/dcevm/jvm.dll`) | GPLv2 | [dcevm/dcevm light-jdk8u181+2](https://github.com/dcevm/dcevm/releases/tag/light-jdk8u181%2B2) |
| HotswapAgent 1.4.1 (`bin/hotswap-agent.jar`) | GPLv2 | [HotswapProjects/HotswapAgent RELEASE-1.4.1](https://github.com/HotswapProjects/HotswapAgent/tree/RELEASE-1.4.1) |
| Apache Maven 3.9.16 (`bin/maven`) | Apache License 2.0 | [apache/maven maven-3.9.16](https://github.com/apache/maven/tree/maven-3.9.16) |

JDK · Maven 의 라이선스 문서는 각 폴더 안에 있다.
