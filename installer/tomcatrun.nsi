; tomcatrun 설치 파일 — 실행기 폴더(톰캣 설정 원본 · DCEVM JDK 8 · HotswapAgent · Maven · 도우미)를 깐다.
; 만들기: installer/build.sh (stage/ 를 채운 뒤 이 스크립트를 makensis 로 돌린다)
;
; 실행기는 exe 위에서 base/conf/server.xml 이 있는 폴더를 찾아 작업 폴더로 쓴다.
; 그래서 exe 만 따로 깔지 않고 폴더째 깐다.
; 설정 파일(tomcatrun.json)은 넣지 않는다. 처음 뜰 때 빈 서버 하나로 만들고, 프로젝트 폴더 · 포트 · 접속 주소는 사용자가 「설정」 탭에서 정한다.
;
; 이미 깔린 폴더(tomcatrun.json 이 있는 곳)에 다시 깔면 「업데이트」다 — 설정 · 의존 jar · 컴파일 결과 · 로그는 그대로 두고 나머지를 바꾼다.
; base/conf 는 바꾼다. 경로 · 포트는 설정에서 속성으로 넘기므로 conf 에는 사용자 값이 없다.

Unicode true
SetCompressor /SOLID lzma
SetCompressorDictSize 64
ManifestDPIAware true

!ifndef VERSION
  !define VERSION "0.1.0"
!endif
!define APP      "tomcatrun"
!define UNINST   "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"
!define STAGE    "stage"

Name "${APP} ${VERSION}"
OutFile "dist\${APP}-setup-${VERSION}.exe"
Icon "..\app\icons\icon.ico"
UninstallIcon "..\app\icons\icon.ico"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\${APP}"
BrandingText "${APP} ${VERSION}"

!include "MUI2.nsh"
!include "LogicLib.nsh"

!define MUI_ICON   "..\app\icons\icon.ico"
!define MUI_UNICON "..\app\icons\icon.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP}.exe"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Korean"

Section "tomcatrun"
  SetOutPath "$INSTDIR"

  ; 실행 중인 exe 는 지워지지 않는다 — 덮기 전에 닫게 한다
  retry:
  ClearErrors
  ${If} ${FileExists} "$INSTDIR\${APP}.exe"
    Delete "$INSTDIR\${APP}.exe"
    ${If} ${Errors}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${APP} 이 실행 중입니다. 창을 닫은 뒤 다시 시도를 누르세요." IDRETRY retry
      Abort
    ${EndIf}
  ${EndIf}

  File "${STAGE}\${APP}.exe"
  File "${STAGE}\README.md"
  File /r "${STAGE}\helper"
  File /r "${STAGE}\overlay"
  File /r "${STAGE}\bin"
  File /r "${STAGE}\base"

  ; 첫 서버가 쓰는 빈 폴더 — 톰캣이 기동 때 쓰는 것 · 실행기가 채우는 컴파일 결과 · 의존 jar
  CreateDirectory "$INSTDIR\classes"
  CreateDirectory "$INSTDIR\deps"
  CreateDirectory "$INSTDIR\base\logs"
  CreateDirectory "$INSTDIR\base\temp"
  CreateDirectory "$INSTDIR\base\work"
  CreateDirectory "$INSTDIR\base\webapps"

  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\${APP}.exe" "" "$INSTDIR\${APP}.exe" 0
  CreateDirectory "$SMPROGRAMS\${APP}"
  CreateShortcut "$SMPROGRAMS\${APP}\${APP}.lnk" "$INSTDIR\${APP}.exe" "" "$INSTDIR\${APP}.exe" 0
  CreateShortcut "$SMPROGRAMS\${APP}\제거.lnk" "$INSTDIR\uninstall.exe"

  WriteRegStr HKCU "${UNINST}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST}" "DisplayIcon" "$INSTDIR\${APP}.exe"
  WriteRegStr HKCU "${UNINST}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "${UNINST}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST}" "NoRepair" 1

  ; 톰캣은 설치 파일에 넣지 않았다. 기본 자리에 없으면 알리기만 한다.
  ${IfNot} ${FileExists} "$PROGRAMFILES64\Apache Software Foundation\Tomcat 8.5\lib\catalina.jar"
    MessageBox MB_ICONINFORMATION "설치는 끝났지만 톰캣 8.5 가 기본 자리(C:\Program Files\Apache Software Foundation\Tomcat 8.5)에 없습니다.$\r$\n$\r$\n톰캣 8.5 를 설치하거나, 실행기 「설정 › 경로」에서 톰캣 폴더를 정해 주세요."
  ${EndIf}
SectionEnd

; 설치 폴더를 통째로 지우지 않는다 — 사용자가 다른 것이 든 폴더를 골랐을 수 있다. 실행기가 만든 것만 지운다.
Section "Uninstall"
  Delete "$DESKTOP\${APP}.lnk"
  RMDir /r "$SMPROGRAMS\${APP}"

  Delete "$INSTDIR\${APP}.exe"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\tomcatrun.json"
  Delete "$INSTDIR\tomcatrun.json.bak"
  RMDir /r "$INSTDIR\base"
  RMDir /r "$INSTDIR\servers"
  RMDir /r "$INSTDIR\bin"
  RMDir /r "$INSTDIR\deps"
  RMDir /r "$INSTDIR\classes"
  RMDir /r "$INSTDIR\helper"
  RMDir /r "$INSTDIR\overlay"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegKey HKCU "${UNINST}"
SectionEnd
