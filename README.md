# tomcatrun

로컬 톰캣 개발 실행기 (Windows).

IDE 의 서버 배포 없이 Spring · 서블릿 웹앱을 로컬 톰캣으로 띄우고, 고친 것을 재시작 없이 반영한다.
프로젝트마다 톰캣 하나를 등록해 따로 켜고 끄며, 여럿을 함께 띄울 수 있다.

- JSP · JS · CSS 는 저장하면 바로, 자바는 저장하면 바뀐 파일만 컴파일해 실행 중인 JVM 에 교체한다 (DCEVM · HotswapAgent).
- 무엇이 바로 들어갔고 무엇이 재시작해야 들어가는지를 「반영 기록」과 알림 줄로 보여 준다.
- 프로젝트 소스 · IDE 설정은 읽기만 한다. 실행기가 쓰는 것은 모두 실행기 폴더 안이다.
- 버전 관리 작업복사본도 기본은 읽기만 한다. 서버마다 「Allow write」를 켜면 그 서버에서만 Commit · Update · Revert · Resolve (Git 은 Fetch · Push 도)를 실행기 안에서 한다.

## 설치

1. [릴리스](../../releases)에서 `tomcatrun-setup-<버전>.exe` 를 받아 실행한다. 기본 위치는 `%LOCALAPPDATA%\Programs\tomcatrun` 이고(이미 깔려 있으면 그 자리) 관리자 권한이 필요 없다.
2. **톰캣 8.5** 를 따로 설치한다 (기본 자리 `C:\Program Files\Apache Software Foundation\Tomcat 8.5`). 다른 자리면 「설정 › 경로」에서 정한다.

설치 파일에는 실행기 · DCEVM JDK 8 (AdoptOpenJDK 8u181 + DCEVM) · HotswapAgent · Maven 이 들어 있다. JDK 를 따로 깔지 않아도 된다.

## 시작하기

처음 켜면 「시작하기」 창이 뜬다.

- **이클립스를 쓰고 있었다면** — 작업 공간을 알아서 찾아 쓰던 톰캣 서버를 보여 준다. 「가져오기」 한 번이면 서버 이름 · 프로젝트 폴더 · 포트 · 접속 경로 · 파라미터 수 상한 · 자바 소스 인코딩 · `-parameters` · 톰캣 폴더 · 의존 jar(이클립스 배포본의 `WEB-INF/lib`)까지 들어온다. 바로 「시작」을 누르면 된다. 이클립스 설정은 읽기만 한다.
  - 이클립스 서버 설정에 JNDI(`<Resource>` · `<Environment>` · `<ResourceLink>`, 예: DB 연결)가 있으면 서버 줄에 「JNDI n개」가 붙는다. 「JNDI 설정 n개도 옮기기」를 켜야 그 서버의 `<톰캣 설정 폴더>/conf/Catalina/localhost/context.xml.default` 로 옮겨진다. 기본은 꺼져 있다. 이 파일에는 DB 계정 · 비밀번호가 그대로 들어가지만, 서버마다 따로 있어 다른 서버나 저장소에는 들어가지 않는다. `ResourceLink` 는 가리키는 전역 설정을 찾아 그 앱의 `<Resource>` 로 바꿔 적는다. 켤 때 저절로 가져오면 JNDI 는 옮기지 않고 알림만 띄운다. 옮기지 않으면 앱이 그 이름을 찾지 못해 기동이 실패할 수 있다.
  - 실행기는 이클립스와 다르게 도는 곳이 있다. 그런 곳을 찾으면 서버 줄 아래에 노란 경고로 보이고, 가져온 뒤에는 「상태」 탭 서버 점검의 「이클립스와 다름」에 남는다. HTTP Connector 속성(maxPostSize · URIEncoding · relaxedQueryChars) · Classpath 탭 · 환경 변수 · 소스 인코딩과 `-parameters`(서버 설정과 다를 때만 — 가져올 때 이클립스 값으로 채운다) · 파일별 인코딩 · 자바 compliance · 배포 폴더 · 함께 배포하는 프로젝트 · pom 리소스 필터링을 본다. 경고마다 무엇을 하면 되는지가 붙는다. 서버 설정(server.xml · 실행 설정) 쪽은 가져올 때 한 번 읽으므로, 이클립스에서 고쳤다면 다시 가져와야 바뀐다. 프로젝트 `.settings` 쪽은 점검할 때마다 다시 읽는다. 리소스 필터링은 가져온 뒤 「리소스 필터링」 줄이 따로 센다 (아래 「무엇이 어떻게 반영되나」).
  - 이클립스 서버가 쓰던 JRE 도 가져와 그 서버의 「java (핫리로드 끔)」 칸에 채운다. 실행 설정에 고른 JRE 가 먼저이고, 없으면 서버 런타임에 고른 JRE, 그다음 작업 공간 기본 JRE 순서로 찾는다. 핫리로드 모드는 늘 함께 까는 DCEVM 8u181 로 뜬다. 그래서 이클립스가 다른 자바를 썼으면 「이클립스와 다름」 점검과 위쪽 알림에 「이클립스가 쓰던 자바는 X 입니다 · 핫리로드 모드는 8u181 입니다」가 뜬다. 알림의 「핫리로드 끄기」로 바로 끌 수 있다. 다시 가져오면 손으로 고친 java 칸도 이클립스 값으로 바뀐다.
  - 함께 배포하는 다른 프로젝트(공통 모듈)는 가져올 때 이클립스 배포본의 jar 로 한 번 옮겨질 뿐이다. 그 프로젝트를 고쳤으면 이클립스에서 다시 배포한 뒤 「자동으로 채우기」(의존 jar 알림) 또는 다시 가져오기로 jar 를 바꾼다.
  - 프로젝트 `WEB-INF/lib` 과 의존 jar 폴더에 같은 라이브러리가 판만 다르게 있으면(예: `poi-3.9.jar` 와 `poi-3.10.jar`) 「상태」 탭 서버 점검에 「같은 jar 다른 판」 노란 줄과 위쪽 경고 알림이 뜬다. 두 판이 함께 올라가 이클립스와 클래스를 읽는 순서가 달라지므로, 이 실행기에서만 NoSuchMethodError · ClassCastException 이 날 수 있다. 알림이나 점검 줄의 「겹친 판 치우기」를 누르면 의존 jar 폴더 쪽 다른 판을 옆 폴더 `deps.dup` 로 옮긴다. 남기는 판은 프로젝트 `WEB-INF/lib` 에 있으면 그 판, 없으면 가장 높은 판이다. 알림의 「자세히」에서 라이브러리마다 남기는 것과 치우는 것을 미리 볼 수 있다. 한 번 치우면 의존 jar 를 다시 받아도 다시 치우고, 「치운 jar」 줄의 「되돌리기」로 돌려놓는다. 둘 다 톰캣을 끈 뒤에 된다. 프로젝트 `WEB-INF/lib` 안에서 판이 겹치면 실행기는 소스 폴더를 건드리지 않고 지울 파일 이름만 알린다. 소스 저장소에서 지우거나 pom.xml 의 판을 맞춘다. 같은 라이브러리인지는 파일 이름에서 판 번호(마지막 「-숫자」부터)를 떼어 가린다. 그래서 이름을 바꿔 둔 jar 는 잡지 못하고, 이름만 같은 다른 라이브러리(`antlr-2.7.7` 과 `antlr-3.5`)는 겹친다고 나온다. 플랫폼만 다른 jar(`…-linux-x86_64` · `…-osx-…`)는 겹침으로 세지 않는다. 경고는 시작을 막지 않는다.
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
| 서버 › 소스 인코딩 | 자바 소스를 읽는 인코딩 (`javac -encoding`). 비우면 UTF-8. 이클립스 프로젝트의 Text file encoding 과 다르면 소스의 한글 문자열이 깨진다 — 컴파일 오류 없이 한글로 비교하는 분기 · 메시지 · 쿼리가 실행 중에만 틀어진다. 「시작하기」가 프로젝트 `.settings` → 작업 공간 기본 순으로 채우고, 어디에도 없으면 UTF-8 로 두고 로그에 알린다 |
| 서버 › 매개변수 이름 | 메서드 매개변수 이름을 클래스에 남긴다 (`-parameters`, 이클립스 「Store information about method parameters」). 켜야 `@Param` 없이 쓴 MyBatis 매퍼가 「Parameter 'x' not found」 없이 돈다. 「시작하기」가 이클립스 값으로 채운다. 소스 인코딩과 이 칸은 톰캣을 끈 뒤에만 바꿀 수 있고(켜진 서버에서는 잠긴다), 바꾸면 다음 기동 · 컴파일 때 컴파일 결과를 비우고 전체를 다시 만든다 |
| 서버 › 접속 경로 | 앱을 여는 경로 (예: `/shop`). 비우면 `/`. 이클립스에서 `/프로젝트명` 으로 열던 앱은 같게 적어야 절대경로 링크 · 리다이렉트가 맞다. `/` 가 아니면 `http://localhost:<포트>/` 는 404 이고, 앞단 Apache(mod_jk)가 루트로 넘기던 구성이면 그쪽 경로도 맞춰야 한다. 「시작하기」가 이클립스 값으로 채운다 |
| 서버 › 파라미터 수 상한 | 요청 하나에 받는 파라미터 수 (톰캣 `maxParameterCount`). 넘으면 나머지가 잘린다 — 그리드 일괄 저장처럼 칸이 많은 요청. 기본 10000 은 톰캣 8.5 기본이자 이클립스 서버에 값이 없을 때와 같다. `-1` 은 무제한. 「시작하기」가 이클립스 값으로 채운다 |
| 서버 › 바로가기 | 서버 줄 「바로가기」 메뉴. 「바로가기 추가」로 이름 · 주소를 한 줄씩 더한다. `/` 로 시작하는 주소(`/` · `/main.do`)는 그 서버의 HTTP 포트와 접속 경로로 열린다 (`/main.do` → `http://localhost:8080/shop/main.do`). 이미 접속 경로로 시작하는 주소(`/shop/main.do`)에는 다시 붙이지 않는다 |
| 서버 › java (핫리로드 끔) | 핫리로드를 끄고 띄울 때 그 서버가 쓰는 `java.exe`. 「시작하기」가 이클립스 서버의 JRE 로 채운다. 비우면 경로 › java 를 쓴다. 칸 아래에 지금 뜰 자바 판과 이클립스가 쓰던 판이 보인다 |
| 서버 › JVM 인자 | 그 서버에만 더할 인자. 회사 · 팀 전용 시스템 속성(`-D…`)은 여기 둔다 |
| 경로 › 톰캣 폴더 · javac · java · 편집기 | 기본은 설치된 JDK 8 · `code`(VS Code). java 는 서버 › java 칸을 비운 서버가 쓴다 |
| 경로 › TLD jar | 태그 라이브러리를 찾을 jar (쉼표로). 비우면 모든 jar 를 본다 |
| 경로 › Commit message 생성 명령 | 「자동 생성」이 부르는 언어 모델 CLI (예: `claude -p`). 기본은 비어 있고, 비우면 자동 생성을 쓰지 않는다 |
| 버전 관리 | 서버마다 SVN · Git · 작업 폴더 · Allow write · Commit message 예시 · 생성 지침 · 제목 규칙 · Update · Pull 명령 · svn.exe 경로 · svn 받기 |
| 저장 감시 제외 | 저장해도 반영 기록 · 재시작 알림에 넣지 않을 파일 · 폴더 |

**실행기가 알아서 정하는 것은 없다.** 서버 이름 · 프로젝트 폴더 · 바로가기는 PC 마다 사람이 정한다 — 어느 폴더가 어느 망(내부망 · 외부망 등)의 소스인지 실행기는 감지하지 않는다.
포트만 기본값이 있다. n 번째 서버(0부터)는 HTTP `8080+n` · AJP `8009+n` · 종료 `8005+100n` · 디버거 `8000+n` 이고, 다른 서버나 지금 이 PC 에서 듣고 있는 프로그램(IDE 톰캣 등)과 겹치면 빈 자리까지 올린다. 새 서버의 바로가기는 `/` 하나다.

「서버 추가」로 서버를 더한다(바로 보인다). 더한 서버의 톰캣 설정 · 컴파일 결과 · 의존 jar 는 `servers/<id>/` 아래에 생긴다.
SVN 저장소 주소는 작업복사본(`.svn/wc.db`)에서 읽고, 로그인이 필요하면 처음 한 번 묻는다. 계정은 **Windows 자격 증명 관리자**(`tomcatrun:svn:<호스트>`)에만 두고 화면 · 로그 · 설정 파일에 남기지 않는다.

> `tomcatrun.json` 은 그 PC 의 설정이다. 회사 내부 경로 · 주소가 들어가므로 공개 저장소에 올리지 않는다 (이 저장소의 `.gitignore` 도 뺀다).

그다음 서버 줄의 「의존 jar 다시 받기」로 `pom.xml` 의 의존 jar 를 받고(사내 Maven 저장소가 필요하면 `%USERPROFILE%\.m2\settings.xml` 에 둔다), 「시작」을 누른다.

## 업데이트

켤 때와 1시간마다, 그리고 마지막 확인에서 10분이 지난 뒤 창으로 돌아올 때 [릴리스](../../releases)의 최신 판을 확인한다. 새 판이 있으면 알림 줄에 「업데이트」 단추가 뜬다.
GitHub 에 닿지 않았거나 새 판의 설치 파일을 아직 만드는 중이면 5분 뒤 다시 확인하고, 거듭되면 10 · 20 · 40분으로 늘려 1시간까지 간다.
누르면 설치 파일을 받아 SHA-256 과 서명을 대조하고, 켜진 톰캣을 정상 종료한 뒤 같은 폴더에 조용히 설치하고 실행기를 다시 연다.
설정 · 의존 jar · 컴파일 결과는 그대로 남는다. 「설정 › 업데이트」에서 자동 확인을 끄거나 지금 확인한다.

서명 비밀 키는 `~/.tauri/tomcatrun.key`(비밀번호 없음)와 저장소 비밀 `TAURI_SIGNING_PRIVATE_KEY` 에 있다. **키를 잃으면 이미 깔린 실행기는 새 판을 받지 못한다.**
GitHub API 한도에 걸리면 API 를 쓰지 않는 주소로 판을 알아낸다. 이때는 릴리스 본문이 보이지 않는다.

GitHub 연결은 Windows 인증서 저장소를 쓰고 `HTTPS_PROXY` 환경 변수를 따른다. 사내망에서 닿지 않으면 조용히 넘어간다(설정 › 업데이트에만 보인다).

## 무엇이 어떻게 반영되나

| 고친 것 | 반영 | 까닭 |
| - | - | - |
| JSP | 저장 후 1초 안 | 톰캣이 `src/main/webapp` 을 직접 읽는다 · JSP 확인 주기 1초 |
| JS · CSS | 새로고침하면 바로 | 브라우저가 매번 서버에 다시 확인한다 (`conf/web.xml` 의 DevNoCache) |
| 매퍼 XML (스스로 다시 읽는 묶음) | 저장 후 몇 초 | 앱의 `RefreshableSqlSessionFactoryBean` 이 원본을 보고 그 묶음을 다시 해석한다. 「매퍼 하나만 다시 읽기」(실험)를 켜면 바뀐 파일만 |
| 자바 — 메서드 안 내용 | 저장 후 1초 안 | 바뀐 파일만 상주 ECJ 로 컴파일 → HotswapAgent 가 실행 중인 JVM 에 교체 · 반영 기록에 「자바 교체」 |
| 자바 — 컨트롤러 · 서비스 같은 빈의 새 URL · 메서드 · 주입 필드 | 저장 후 10초 안팎 | 교체 뒤 Spring 이 빈을 다시 등록하고 URL 매핑을 새로 만든다. 끝나면 반영 기록에 「Spring 반영」 이 붙는다. 그 전에 누르면 옛 매핑이 응답한다. 톰캣이 켜져 있으면 컴파일 결과를 임시 폴더(`<컴파일 결과 폴더>.stage`)에 모았다가 「컴파일 더함」까지 끝난 뒤 한 번에 내놓으므로, 여러 파일이 바뀌어도 교체와 빈 다시 등록은 한 번씩이다 |
| 자바 — 상수 값 · 메서드 시그니처 | 저장 후 몇 초 | 그 상수 · 메서드를 쓰는 소스도 함께 컴파일한다 (반영 기록 「컴파일 더함」). 자바는 다른 클래스의 상수를 값으로 박아 넣어서, 바뀐 파일만 컴파일하면 쓰는 쪽에 옛 값이 남는다 |
| 자바 — 상속 · 인터페이스 · 안쪽 · 익명 클래스 구조 | 재시작 | 교체가 실패하면 재시작 알림에 올라온다 |
| 자바 — static 초기값 · 생성자 · `@PostConstruct` · 필드 초기값 | 재시작 | 클래스는 바뀌어도 이미 만들어진 객체의 초기화는 다시 돌지 않는다. 실행기가 가려내지 못해 알림이 뜨지 않는다 |
| 자바 소스 지움 · 옮김 | 재시작 | 컴파일 결과의 클래스를 지우고 재시작 알림에 올린다 (켜진 앱에는 남아 있다) |
| Spring 설정 · XML 로 정의한 빈 · web.xml · 그 밖의 매퍼 · 메시지 · pom 의존성 | 재시작 | 기동 때 한 번만 읽는다 |

재시작이 필요한 것은 서버 줄 아래 노란 알림 줄에 서버별로 모이고 그 서버의 재시작 단추에 개수가 붙는다. HotswapAgent 가 교체에 실패해도 여기에 올라온다.
HotswapAgent 의 원문 줄(`Reloading classes …` · `Registering Spring bean …`)은 서버 로그 탭에만 남고, 반영 기록에는 「자바 교체」 · 「Spring 반영」 두 줄로 묶여 보인다.
스스로 다시 읽는 매퍼 묶음은 프로젝트의 `src/main/resources/spring/context-datasource.xml` 에서 `RefreshableSqlSessionFactoryBean` 의 `mapperLocations` 를 읽어 정한다 (서버를 켤 때마다).

실행기는 `src/main/resources` 를 Maven 처럼 걸러 넣지 않고 원본 그대로 읽는다. 그래서 pom 에 `<filtering>true</filtering>` 가 있으면 이클립스(Maven)가 값으로 바꾸는 `${key}` · `@key@` 자리에 글자가 그대로 들어간다. 「상태」 탭 서버 점검의 「리소스 필터링」 줄은 그런 자리가 몇 곳, 어느 파일에 있는지 보여 준다. 세는 것은 Maven 이 값을 아는 키뿐이다(pom · 프로필 `<properties>` · `<filters>` 파일 · `project.*` · `env.*`). Spring 이 기동할 때 채우는 `${db.url}` 같은 키는 세지 않는다. 프로필은 이클립스가 쓰는 것을 따른다(프로젝트 `.settings/org.eclipse.m2e.core.prefs` 의 `activeProfiles`, 없으면 `activeByDefault`). 조건으로 켜지는 프로필이나 상위 pom 의 속성처럼 판단하지 못한 것은 「판단 못 함」으로 덧붙인다. pom 에 필터링이 없으면 이 줄은 보이지 않는다.

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
| 클래스 전체 탐색 건너뛰기 | 끔 | web.xml 사본에 `metadata-complete="true"` 와 빈 `<absolute-ordering/>` 를 더하고 웹소켓 초기화를 거른다. 서블릿 3.0 애너테이션 · 초기화 클래스를 쓰지 않는 앱에서만 켠다. 켜도 되는지는 「상태」 탭의 「기동 탐색」 줄이 알려 준다 (아래) |
| 서버를 차례로 띄우기 | 켬 | 먼저 누른 서버가 다 뜬 뒤 다음 서버를 띄운다 |
| 기동 표본 남기기 | 끔 | 켜는 동안 2초마다 jstack 으로 기동 스레드를 떠서, 어디에 오래 머물렀는지 서버의 `logs/startup-profile-<시각>.txt` 에 남긴다. 뜨는 동안 JVM 이 잠깐씩 멈춰 기동이 조금 늘어나므로 잴 때만 켠다 |
| DB 연결 미리 열기 1개 | 켬 | commons-dbcp `BasicDataSource` 의 처음 연결 수를 1로 |
| 예약 작업 멈추기 | 끔 | `@Scheduled` 를 돌리지 않는다 |
| 지연 초기화 (실험) | 끔 | 빈을 처음 쓸 때 만든다 |
| 매퍼 하나만 다시 읽기 (실험) | 끔 | HotswapAgent MyBatis 플러그인 · 앱의 묶음 감시를 끈다 |
| 재시작해도 로그인 유지 | 켬 | 정상 종료 때 세션을 저장했다가 되살린다 (톰캣 기본 동작) |

**개발 전용 앱 설정** (`overlay/`): `tomcatrun.DevTweaks` 가 실행기가 주는 시스템 속성(`tomcatrun.*`)에 따라 Spring 빈 정의만 고친다. 속성이 없으면 아무것도 하지 않는다.
앱이 `classpath*:spring/context-*.xml` · `classpath*:egovframework/springmvc/*.xml` 로 설정을 읽는 구성(전자정부 표준프레임워크 꼴)에서 원본과 함께 읽힌다. 다른 구성이면 위 네 옵션(DB 연결 · 예약 작업 · 지연 초기화 · 매퍼 하나만)이 동작하지 않을 뿐 나머지는 그대로다.

**기동 단계 측정**: 기동이 끝나면 반영 기록에 단계별 시간과 GC 시간 · 횟수, 읽은 클래스 수 · 그 시간이 남고, `logs/startup-history.csv` 에 기동마다 한 줄씩 쌓인다. GC · 클래스 수치는 띄운 java 옆의 `jstat` 으로 재므로 JRE 로 띄우면 빈칸이다. 열이 바뀐 옛 파일은 `startup-history.prev.csv` 로 옮겨 둔다. 「기동 표본 남기기」를 켜고 띄운 기동은 옵션 열에 「표본」이 붙는다. 표본 파일에는 「맨 위」(그 자리에서 머문 곳) · 「포함」(그 아래 호출까지) · 「단계별 맨 위」가 횟수와 비율로 적힌다. JDK 안쪽 프레임은 건너뛰고 그것을 부른 쪽으로 센다.

**기동 탐색 점검**: 「상태」 탭 서버 점검의 「기동 탐색」 줄은 jar 목록과 소스를 읽어 두 설정을 견준다. 「TLD jar」 칸이 비어 모든 jar 를 훑고 있으면 태그 라이브러리가 든 jar 를, 「클래스 전체 탐색 건너뛰기」가 꺼져 있는데 켜도 되면 그렇다고 알린다. 켜도 되는 것은 소스에 `@WebServlet` · `@WebFilter` · `@WebListener` · `@HandlesTypes` · `WebApplicationInitializer` · `ServletContainerInitializer` 가 없고, jar 에 spring-web 말고는 web-fragment · 초기화 클래스가 없을 때다. 「탐색 줄이기」를 누르면 모든 서버의 TLD jar 를 칸에 모으고, 모든 서버가 켜도 될 때만 건너뛰기를 켠다 (두 값은 모든 서버가 함께 쓴다). 다음 기동부터 들어간다. 저절로 바꾸지는 않는다. 이름을 바꾼 jar 나 리플렉션으로 등록하는 서블릿처럼 여기서 못 보는 길이 있어서다. 줄여도 되는 서버가 있으면 맨 위 알림 줄에도 「기동 탐색을 줄이면 빨리 뜹니다」가 같은 단추와 함께 뜬다(이번 실행에서 띄운 기동의 웹앱 준비 시간이 있으면 함께). 「닫기」를 누르면 판정이 바뀌기 전까지 다시 뜨지 않는다. TLD jar 칸에 빠진 jar 가 있거나 건너뛰기가 그 서버에 맞지 않으면 닫을 수 없는 경고로 뜬다.

**알림 줄**: 위에서부터 오류(빨강) · 손볼 일(호박) · 제안(무채색 · 기동 탐색 줄이기와 새 판) 순서다. 세 줄까지만 펼치고 나머지는 「알림 n개 더」 한 줄로 접는다(오류는 늘 모두). 「이클립스와 다름」 · 「리소스 필터링」도 상태 탭에만 두지 않고 알림 줄에 뜬다. 알고 쓰는 차이일 수 있어 닫을 수 있고, 닫으면 내용이 바뀌기 전까지 다시 뜨지 않는다(상태 탭에는 남는다). 칸에 적힌 패턴이 TLD 가 든 jar 를 빠뜨리거나, 건너뛰기가 켜져 있는데 건너뛰면 빠지는 것이 있으면 같은 줄이 노란 경고로 알린다.

## 버전 관리 (SVN · Git)

「버전 관리」 탭. 서버마다 SVN · Git · 둘 다 · 안 씀을 고른다 (기본은 작업 폴더의 `.svn/wc.db` · `.git` 을 보고 자동).
동작 · 상태 이름은 TortoiseSVN · Git 과 같은 영문 표준 용어를 쓴다 (Commit · Update · Revert · Modified · Conflicted …).

화면은 위에서 아래로 저장소 줄 · 띠 · 왼쪽 목록과 Commit 칸 · 오른쪽 Diff 다.

| 자리 | 무엇 |
| - | - |
| 저장소 줄 | 서버 · 저장소 경로 · 리비전, 받을 커밋 수, Update(Git 은 Fetch · Pull · Push) 단추. 쓰기를 끈 서버는 명령 복사 단추 |
| 띠 | 지금 손댈 것 — 쓰기 실패(무엇을 하면 되는지와 svn 원문), 남은 작업복사본 잠금, 쓰기를 못 하는 까닭. 도는 동안은 걸린 시간 |
| Changes | 맨 위에 받을 커밋(Update 하면 들어올 것 · 내 변경과 겹치는 것), 그 아래 Conflicted · Changes · Unversioned 묶음. 파일 찾기 · 모두 고르기 · 화살표와 Space 로 고르기 |
| History | 최근 커밋. 고르면 오른쪽에 메시지 · 바뀐 파일 · 그 Diff |
| Commit 칸 | 제목 · 설명 · 자동 생성 · Commit 단추. 누를 수 없으면 그 까닭이 단추 아래에 뜬다. Ctrl+Enter 로 Commit 한다 |

받을 커밋은 그 화면을 열 때 저장소에 묻고 5분 동안 그대로 쓴다. 계정이 없으면 로그인 칸을 띄우지 않고 저장소 줄에 적기만 한다.

**저장소에 쓰기 (Allow write)** — 「설정 › 버전 관리」에서 서버마다 켠다. 기본은 꺼짐이고, 끈 서버는 명령을 복사만 해 준다.

**파일 트리 · 검토** — Changes 는 폴더 트리로 보이고(머리 줄 단추로 목록으로 바꾼다), 파일 종류는 색 있는 아이콘으로 가린다. Commit · Update · Pull 은 창 대신 본 화면의 검토 단계에서 Diff 를 보고 고른다. Ctrl+K 로 파일과 명령을 찾는다.

**복사본** — Revert · Resolve · Delete · Revert to rN · Delete unversioned 와 Update · Switch · Merge · Apply patch 전에 내 파일을 `%LOCALAPPDATA%\tomcatrun\vcs-backup` 에 14일 둔다(파일 하나 100MB 까지). 알림의 「되살리기」나 더 보기의 「복사본에서 되살리기」로 되돌린다. 로컬 설정 파일도 그대로 복사되니 비밀번호가 든 파일은 이 폴더에도 남는다. 복사본을 못 만들면 쓰기를 멈춘다.

**Commit 에서 늘 빼기** — 줄 메뉴에서 정한다. TortoiseSVN 과 같은 `ignore-on-commit` changelist 라 두 도구에서 같은 파일이 빠진다.
켜면 아래를 실행기 안에서 한다. 실행 전에 늘 확인 창이 뜨고 넘길 파일 목록을 보여 준다.

| 동작 | SVN | Git |
| - | - | - |
| Commit | 고른 파일만 (`--depth empty` · 경로는 `--targets` 파일로). Unversioned 는 먼저 `svn add`, Missing 은 `svn delete`. 지운 폴더가 끼면 그 폴더째 `--depth infinity` 로 올린다 | `git add -A` 뒤 `git commit -- 고른 파일` |
| Update · Pull | `svn update` (겹치면 Conflicted 로 남는다). 검토 단계에서 받을 파일을 폴더별 · 커밋별로 고른다. 줄마다 올린 사람 · 리비전이 보이고, 줄을 누르면 오른쪽에 커밋 메시지 전문과 `BASE → HEAD` Diff 가 보인다. 받은 결과는 Changes 맨 위 「방금 받음」 에 남는다 | 설정의 Pull 명령 (기본 `git pull --ff-only`). 검토 단계에 받을 커밋과 올린 사람 · 일시가 보이고, 줄을 누르면 오른쪽에 메시지 전문과 받을 파일이 보인다 |
| Fetch | 없음 | `git fetch --prune`. 원격 추적 브랜치만 새로 고치고 작업 파일은 그대로라 확인 창 없이 돈다 |
| Push | 없음 (Commit 이 곧 올리기) | 지금 브랜치를 원격 추적 브랜치(`@{u}`)에만 올린다. 확인 창에 올라갈 커밋을 보인다. 원격 추적 브랜치가 없는 새 브랜치는 올리지 않는다 (`git push -u` 는 직접). `--force` 는 쓰지 않는다 |
| Revert | BASE 로 되돌린다. Added 는 Unversioned 로 돌아가고 파일은 남는다 | HEAD 로 되돌린다. Added 는 색인에서만 빠진다 |
| Resolve | Use mine · Use theirs · Mark as resolved. BASE ↔ mine · BASE ↔ theirs 를 나란히 본다 | 같은 세 가지 (`--ours` · `--theirs` · `git add`) |
| Cleanup | `svn cleanup` (옵션 없음 · 잠금과 끊긴 작업만 푼다. 고친 파일 · Unversioned 는 그대로) | 없음 |
| 리비전 되돌리기 (History) | `svn merge -c -N` · 결과는 Changes 에 남고 Commit 은 따로 | `git revert --no-commit` |

- Conflicted 가 남은 파일은 Commit 하지 않는다. 내가 고친 파일과 겹치는 리비전은 되돌리지 않는다. 고른 파일 내용이 모두 BASE 와 같아 올라간 것이 없으면 성공이라 하지 않고 그렇게 알린다.
- **작업복사본 잠금** — svn 이 쓰는 도중 끊기면(IDE 의 SVN 이 죽거나, 쓰는 중에 프로그램을 닫거나) `.svn/wc.db` 에 잠금이 남아 Commit · Update 가 `E155004` 로 실패한다. 실행기는 쓰기 전에 잠금을 먼저 보고, 남아 있으면 svn 을 부르지 않고 띠에 잠긴 폴더와 Cleanup 단추를 보인다. Eclipse(Subversive · Subclipse 는 svn.exe 없이 같은 작업복사본을 잠근다) · TortoiseSVN 이 켜져 있으면 그 이름도 함께 적는다. Commit · Update 가 도는 중에 실행기를 닫으면 그 일이 끝난 뒤에 닫힌다.
- 실패하면 svn 오류 번호로 무엇을 하면 되는지를 띠에 먼저 적는다 — 잠금은 Cleanup, 저장소가 더 새로우면(`out of date`) Update, 연결 실패 · 다른 사람의 lock · pre-commit hook 거절 따위.
- 복사하는 명령은 `pushd "<작업 폴더>"` 로 시작하고 svn 은 실행기가 찾은 svn.exe 경로를 쓴다. cmd · PowerShell 어느 쪽에 붙여도 돌고, PATH 에 svn 이 없어도 된다.
- SVN 쓰기 · Blame 은 **svn.exe 1.10 이상**이 필요하다. 못 찾으면 「설정 › 버전 관리」의 **svn 받기** 단추가 VisualSVN 이 배포하는 Apache Subversion 명령줄 도구 zip 을 받아 SHA-256 을 대조한 뒤 `%LOCALAPPDATA%\tomcatrun\svn` 에 푼다 (TortoiseSVN 을 설치할 때 「command line client tools」를 고르거나, 설정에 경로를 적어도 된다). 찾는 순서는 설정 경로 → PATH → TortoiseSVN · SlikSVN → 받은 svn 이다. 판이나 작업복사본 형식이 맞지 않으면 쓰기 단추를 잠그고 까닭을 보인다. `svn upgrade` 는 하지 않는다.
- SVN 계정은 먼저 svn 자신의 인증 캐시를 쓰고, 안 되면 실행기 계정을 `--password-from-stdin --no-auth-cache` 로 넘긴다. 비밀번호는 명령줄 · 로그에 남지 않는다.
- 무엇을 썼는지는 그 서버의 로그에 한 줄씩 남는다.

**Commit message 자동 생성 · 규칙** — SVN · Git 모두, Allow write 와 상관없이 쓴다.

- 「설정 › 경로 › Commit message 생성 명령」에 언어 모델 CLI 를 적으면 메시지 칸 옆에 「자동 생성」이 뜬다. 누르면 고른 파일의 Diff · 그 서버의 생성 지침 · 예시 · 브랜치에서 찾은 이슈 번호를 **stdin** 으로 넘기고, 답의 첫 줄은 제목 칸, 나머지는 설명 칸에 채운다. 칸을 고친 뒤 Commit 한다.
- **고른 파일의 Diff 가 그 CLI 의 서비스로 나간다.** 설정 파일에 든 주소 · 계정도 함께 나갈 수 있다. 명령을 비우면(기본) 아무것도 나가지 않는다. 로그에는 언제 몇 개를 보냈는지만 남고 내용은 남지 않는다.
- Diff 는 파일마다 앞 8KB, 모두 합쳐 60KB 까지 넘긴다. 바이너리 · 3MB 넘는 파일은 이름만 넘긴다. 90초 안에 답이 없으면 끊는다.

| 설정 › 버전 관리 | 무엇 |
| - | - |
| 생성 지침 | 자동 생성 때 모델에게 주는 팀의 글 지침 (자유 글). 칸 아래 경고와는 상관없다 |
| 이슈 번호 찾기 | 브랜치(SVN 은 저장소 경로)에서 이슈 번호를 찾는 정규식. 기본 `[A-Z][A-Z0-9]+-\d+` |
| 제목 규칙 | 제목(첫 줄)이 맞아야 하는 정규식. 예: `^[A-Z]+-\d+ .+다$` |
| 제목 최대 길이 | 제목 글자 수 상한 (한글도 한 글자). 0 이면 보지 않는다 |

규칙은 막지 않고 알린다. 칸 아래에 어긋난 까닭이 뜨고, 그대로 Commit 하거나 Commit 명령을 복사하려 하면 확인 창에서 「그래도 …」를 한 번 더 고른다. 빈 칸은 보지 않는다.

SVN 상태는 `.svn/wc.db` 를 읽기 전용(immutable)으로 열어 계산하고, 저장소 조회는 mod_dav_svn 에 읽기 요청(`OPTIONS` · `REPORT` · `GET`)만 보낸다. Git 조회는 저장소를 바꾸지 않는 명령만 부른다.

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
- 핫리로드 모드는 DCEVM 8u181(2018년 판)의 루트 인증서(cacerts)에 이 PC 의 Windows 루트 인증서 저장소를 더한 목록으로 뜬다. 목록은 기동마다 `<서버 폴더>/temp/cacerts` 에 새로 만든다. 그래서 2018년 뒤에 바뀐 인증서를 쓰는 외부 HTTPS(예: 브이월드 `api.vworld.kr`)도 이클립스처럼 호출된다. 기동 로그에 「인증서 목록: … n개를 더했습니다」가 남는다. 서버 JVM 인자에 `-Djavax.net.ssl.trustStore=` 를 직접 주었으면 그것을 따른다.
- **외부 연계 호출에서 그래도 `SSLHandshakeException` 이 나면 핫리로드를 끄고 띄워 본다.** DCEVM 8u181 에는 TLS 1.3 이 없다. Windows 가 아직 내려받지 않은 루트 인증서도 목록에 없다 — 같은 주소를 Edge 로 한 번 열면 Windows 가 받아 둔다. 더 높은 자바로 만든 jar 의 `UnsupportedClassVersionError` 도 핫리로드를 꺼야 풀린다. 핫리로드를 끄면 서버 › java 칸의 자바(이클립스에서 가져왔으면 이클립스와 같은 JRE)로 뜬다. 핫리로드는 DCEVM 에 묶여 있어 그 모드의 자바는 바꿀 수 없다.
- 실행 자바를 바꿔도 컴파일은 그대로 javac(JDK 8) · ECJ 가 1.8 기준으로 한다. 더 높은 자바로 만든 jar 의 클래스를 소스에서 직접 쓰면 컴파일에서 막힐 수 있다.
- 자바 9 이상으로 띄우면 `-Djava.endorsed.dirs` 를 빼고 톰캣 catalina.bat 와 같은 `--add-opens` 를 더한다 (13 이상은 `-Xverify:none` 도 뺀다). 판은 JDK 의 `release` 파일로, 없으면 폴더 이름(`jdk-17` 등)으로 가린다.

## 폴더

| 폴더 | 무엇 |
| - | - |
| `base/conf/` | 톰캣 설정 한 벌. 경로 · 포트 · 접속 경로 · 파라미터 수 상한을 적지 않고 실행기가 기동 때 `tomcatrun.*` 속성으로 넘긴다. 첫 서버는 `base/` 를, 더한 서버는 `servers/<id>/base` 에 이것을 복사해 쓴다. 더한 서버의 `server.xml` 이 옛 원본 그대로면 기동 때 새 원본으로 바꾸고 `server.xml.bak` 을 남긴다. 고쳐 쓴 파일은 그대로 두고 로그에 경고만 남긴다 |
| `classes/` · `deps/` | 첫 서버의 컴파일 결과 · 의존 jar |
| `bin/` | DCEVM JDK 8 · HotswapAgent · Maven (`installer/fetch.sh` 가 받는다) |
| `helper/` | 상주 컴파일러(`TomcatrunCompiler`) · 개발 전용 앱 설정(`DevTweaks`) 소스 · `build.sh` |
| `overlay/` | 개발 전용 앱 설정. 모든 서버의 `/WEB-INF/classes` 에 덧붙는다 |
| `app/` | 실행기 소스 (Rust · Tauri 2) |
| `installer/` | 설치 파일 (`fetch.sh` · `build.sh` · `tomcatrun.nsi`) |
| `tomcatrun.json` | 이 PC 의 설정 (git 에 넣지 않는다) |

## 소스에서 만들기

Windows · Git Bash · Rust 1.99 이상 · NSIS 가 필요하다. Node.js 는 필요 없다 (화면 파일은 `cargo build` 가 그대로 넣는다).
설치 파일로 쓰는 사람에게는 이것들이 필요 없다 — 개발할 때만 쓴다.

```bash
rustc --version             # 1.99 이상이어야 한다. 낮으면 rustup update stable
```

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
