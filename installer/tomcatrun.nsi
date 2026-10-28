; tomcatrun 설치 파일 — 실행기 폴더(톰캣 설정 원본 · DCEVM JDK 8 · HotswapAgent · Maven · 도우미)를 깐다.
; 만들기: installer/build.sh (stage/ 를 채운 뒤 이 스크립트를 makensis 로 돌린다)
;
; 실행기는 exe 위에서 base/conf/server.xml 이 있는 폴더를 찾아 작업 폴더로 쓴다.
; 그래서 exe 만 따로 깔지 않고 폴더째 깐다.
; 설정 파일(tomcatrun.json)은 넣지 않는다. 처음 뜰 때 빈 서버 하나로 만들고, 프로젝트 폴더 · 포트 · 접속 주소는 사용자가 「설정」 탭에서 정한다.
;
; 이미 깔린 폴더(tomcatrun.json 이 있는 곳)에 다시 깔면 「업데이트」다 — 설정 · 의존 jar · 컴파일 결과 · 로그는 그대로 두고 나머지를 바꾼다.
; base/conf 는 바꾼다. 경로 · 포트는 설정에서 속성으로 넘기므로 conf 에는 사용자 값이 없다.
;
; 실행기의 자동 업데이트는 「설치 파일 /S /D=<실행기 폴더>」 로 조용히 돌린다. 실행기가 톰캣을 끄고 닫히는 동안 exe 가 풀리기를 기다리고,
; 다 깔면 실행기를 다시 띄운다 (.onInstSuccess). 조용히 돌 때 뜨는 창은 없다 (MessageBox 마다 /SD).

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
; 2026.10.28 : $LOCALAPPDATA\tomcatrun → $LOCALAPPDATA\Programs\tomcatrun. Windows 가 사용자별 프로그램 자리로 정한 곳이다.
; 실행기가 자기 폴더에 설정 · 로그 · 컴파일 결과를 쓰므로 Program Files 에는 둘 수 없다.
; 이미 깔린 곳이 있으면 그 자리를 기본으로 잡는다 — 그렇지 않으면 다시 깔 때 새 자리에 한 벌이 더 생긴다.
InstallDir "$LOCALAPPDATA\Programs\${APP}"
InstallDirRegKey HKCU "${UNINST}" "InstallLocation"
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

  ; 실행 중인 exe 는 지워지지 않는다 — 60초 동안 풀리기를 기다리고(자동 업데이트는 실행기가 톰캣을 끄고 닫히는 중이다), 그래도 안 되면 닫게 한다
  StrCpy $0 0
  retry:
  ClearErrors
  ${If} ${FileExists} "$INSTDIR\${APP}.exe"
    Delete "$INSTDIR\${APP}.exe"
    ${If} ${Errors}
      ${If} $0 < 120
        IntOp $0 $0 + 1
        Sleep 500
        Goto retry
      ${EndIf}
      MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${APP} 이 실행 중입니다. 창을 닫은 뒤 다시 시도를 누르세요." /SD IDCANCEL IDRETRY retry
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
    MessageBox MB_ICONINFORMATION "설치는 끝났지만 톰캣 8.5 가 기본 자리(C:\Program Files\Apache Software Foundation\Tomcat 8.5)에 없습니다.$\r$\n$\r$\n톰캣 8.5 를 설치하거나, 실행기 「설정 › 경로」에서 톰캣 폴더를 정해 주세요." /SD IDOK
  ${EndIf}
SectionEnd

; 자동 업데이트(조용한 설치)가 끝나면 실행기를 다시 띄운다. 손으로 깔 때는 마침 화면의 「실행」 칸이 그 몫이다.
Function .onInstSuccess
  IfSilent 0 +2
    Exec '"$INSTDIR\${APP}.exe"'
FunctionEnd

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
