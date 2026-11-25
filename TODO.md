# TODO — Kubernetes và Compose (cập nhật 2026-11-25)

Trạng thái: **chưa commit**. Toàn bộ code Giai đoạn 1 nằm trong working tree. Bản này đã được cài và app đang chạy bản mới.

## Đã xong và đã kiểm tra
- Backend `kind_access.rs`:
  - port API cố định 16443–16999;
  - export/remove kubeconfig sang host (có khôi phục context trước đó);
  - mở host terminal;
  - diff/apply YAML;
  - log nhiều pod theo selector.
- Chọn port API có xét port binding của mọi node kind, kể cả cluster đang dừng hoặc tạo ngoài app (`bound_api_ports`). Đã kiểm tra live: `phase1` giữ 16443 thì cluster mới nhận 16444.
- **Bug tạo cluster qua app/MCP lỗi `kubeadm init`: ĐÃ SỬA.**
  - Nguyên nhân: state containerd cũ của node cùng tên không xoá được. Lý do:
    - overlayfs để lại thư mục `work/work` mode 000;
    - file trong image layer thuộc subuid của engine rootless.
  - Khi tạo lại cluster cùng tên, kubelet chạy lại static pod cũ, nên kubeadm bị timeout.
  - Cách sửa:
    - `remove_state_dir` thử xoá thường trước; nếu lỗi thì chạy `unshare --map-auto --map-root-user rm -rf` (user namespace map giống engine);
    - trước khi tạo, state cũ bị xoá; nếu không xoá được thì báo lỗi rõ ràng.
  - Kiểm tra live: `cluster_create phase2` qua MCP thành công. `cluster_delete` dọn sạch thư mục state.
- Cleanup khi xoá cluster giờ lấy thư mục state từ **mount thật** của node (`/var/lib/containerd`). Mount được kiểm tra chặt theo dạng `…/kind-nodes/<name>/nodeN`. Lý do: cluster tạo trước khi app đổi tên (ví dụ `mycluster`) có state nằm ở `~/.local/share/docker-manager/…`.
- Tab Services có nút Logs theo selector. Viewer YAML tự nạp lại sau khi apply. Kind giữ dòng `ERROR:`. Có patch timeout cho kubeadm.
- Đã xoá các cluster test `phase1`, `phase2`, `phase3`.
- Test: Rust 319 pass, clippy sạch, `npm test` 93 pass, tsc sạch.

## Còn lại
1. **Xem UI bằng mắt** (agent-desktop không đọc được webview và không chụp được màn hình trên Wayland):
   - danh sách cluster dạng hàng một dòng;
   - menu "…";
   - các modal Apply YAML, Logs, kubectl shell;
   - bulk start/stop;
   - mở host terminal.
2. Dọn thư mục state mồ côi. Các thư mục này không còn cluster nào dùng. Tổng khoảng 2.6G, có file thuộc subuid nên phải xoá trong user namespace:
   - `~/.local/share/hsx2dock/dtype-compat/kind-nodes/{cluster,fix3,my-cluster,mycluster}`. Riêng `mycluster` ở đây là bản mồ côi; cluster thật dùng thư mục legacy.
   - `~/.local/share/docker-manager/dtype-compat/kind-nodes/{phase1,phase3}`.
   - Cân nhắc thêm nút "Dọn state mồ côi" trong app.
3. `mycluster` dùng port ephemeral 42129, nên host không truy cập được. Muốn dùng từ host thì phải tạo lại.
4. Hỏi user trước khi commit.

## Giai đoạn 1 — sửa UI/UX
- [x] Toàn app dùng góc vuông: `tailwind.config.js` đặt mọi `rounded-*` = 0; các pill `rounded-full` đổi sang `rounded-none`. Chỉ chấm trạng thái và spinner còn tròn.
- [x] ClusterCard thiết kế lại (đã xem preview ở 1440 / 1200 / 560px):
  - header gồm icon trạng thái màu và icon badge có tooltip;
  - info panel rộng cố định;
  - 4 chart Pods / CPU / Memory / Disk xếp 4 → 2 → 1 cột;
  - hàng nút ở footer.
- [x] Disk = writable layer + thư mục state containerd của node (du, cache 60s), có lịch sử cho chart.
- [x] Panel info dạng ô kẻ chia: Uptime | Nodes sát nhau, API server, Forwarded (bỏ port API trùng). Nodes tô vàng khi thiếu node.
- [x] Auto-start chuyển lên header thẻ. Bỏ chọn nhiều cluster; toolbar chỉ còn Start all / Stop all (áp dụng cho mọi cluster).
- [ ] Xem lại trên app thật các màn khác sau khi đổi sang góc vuông (Dashboard, Containers, Images, Gateway, Settings, các modal). Gom thêm lỗi UI/UX còn lại.

## Giai đoạn 2 — Tạo cluster và node
### 2.1 Wizard tạo cluster nâng cao
- [x] Backend:
  - `create_kind_cluster` nhận `ClusterSpec` gồm `k8s_version`/image, `control_planes` (1 hoặc 3 = HA), `workers`, `ingress_ports` (map 80/443 vào control-plane kèm label `ingress-ready=true`), `extra_mounts` (hostPath→containerPath, ro), `extra_port_mappings`, `pod_subnet`/`service_subnet`, `feature_gates`;
  - sinh kind config bằng hàm thuần, có test;
  - kiểm tra port host 80/443 trùng hoặc bị chiếm; giữ port API cố định 16443–16999 và extraMounts d_type cho mỗi node.
- [x] Danh sách phiên bản: bảng tag `kindest/node` ứng với kind v0.33.0 (có digest). Đánh dấu image đã pull sẵn.
- [x] Preset dựng sẵn và preset của người dùng (lưu trong config):
  - "Dev nhỏ" (1 node);
  - "Ingress" (1 node + 80/443);
  - "HA" (3 CP + 2 worker);
  - "Multi-worker".
- [x] UI wizard nhiều bước:
  1. Cơ bản (tên, version);
  2. Topology (CP/worker);
  3. Mạng (port, subnet);
  4. Mounts;
  5. Xem trước YAML (chỉnh tay được, validate).
  Khi tạo, hiện tiến trình.
- [x] MCP: `cluster_create` nhận spec đầy đủ, `cluster_preview_config`.
- [x] Test: sinh config, validate (trùng port, mount không tồn tại, HA với số CP chẵn).

### 2.2 Local registry dùng chung
- [x] Backend:
  - tạo/khởi động/xoá container `kind-registry` (`registry:2`) trên network `kind`, port host cố định;
  - volume để giữ dữ liệu;
  - trạng thái, dung lượng, danh sách repo/tag qua `/v2/_catalog`.
- [x] Nối registry vào cluster:
  - containerd `config_path` + `hosts.toml` (`localhost:<port>` → `kind-registry:5000`) cho mọi node, cả cluster đã có;
  - ConfigMap `local-registry-hosting` trong `kube-public`.
- [x] Wizard có tuỳ chọn "Dùng local registry" (thêm `containerdConfigPatches`).
- [x] Trang Images: nút "Push vào local registry" (tag rồi push), lọc và xoá image trong registry.
- [x] MCP: `registry_status`, `registry_push_image`, `registry_list`. — tên thực tế: `local_registry_status`, `local_registry_start`, `local_registry_remove`, `local_registry_push`, `cluster_connect_registry`.

### 2.3 Thêm/bớt worker node
- [x] Backend:
  - thêm worker vào cluster đang chạy: tạo container node cùng image/network/labels kind, extraMounts d_type riêng, rồi `kubeadm join` với token từ control-plane;
  - bớt worker: `kubectl drain` → `delete node` → xoá container và state dir;
  - chặn khi cluster đang busy.
- [x] UI: nút +/– node trong ClusterMonitor (tab Nodes), có tiến trình và xác nhận.
- [x] MCP: `cluster_add_worker`, `cluster_remove_worker` (cần confirm). — tên thực tế: `cluster_add_worker`, `cluster_remove_node`.

## Giai đoạn 3 — Mạng và kết nối
### 3.1 Catalog addon
- [x] Mô hình addon: manifest/Helm, phiên bản ghim, bước chờ ready, điều kiện tiên quyết (ingress cần port 80/443 và label `ingress-ready`).
- [x] Các addon:
  - ingress-nginx (bản cho kind);
  - MetalLB (tự tính dải IP từ subnet của network `kind`);
  - cert-manager (thêm ClusterIssuer self-signed);
  - Kubernetes Dashboard (token admin, port-forward).
- [x] Trạng thái từng addon: chưa cài / đang cài / ready / lỗi. Có gỡ cài đặt.
- [x] UI: tab Addons trong ClusterMonitor, dạng thẻ catalog, xem log khi cài.
- [x] MCP: `addon_list`, `addon_install`, `addon_uninstall`.

### 3.2 Domain qua Gateway của app
- [x] Map host `*.local` (hoặc domain tuỳ chọn) qua Gateway (Caddy) tới ingress của cluster (port 80/443 đã map). — `ingress_expose` thêm route Caddy `k8s-<cluster>-<host>` tới cổng ingress 80 của cluster.
- [x] Khi tạo Ingress từ UI: chọn domain, tự thêm route Gateway và bản ghi hosts; TLS qua cert-manager hoặc Caddy internal CA. — Giới hạn: app không sửa `/etc/hosts`. Domain cần DNS hoặc hosts trỏ về 127.0.0.1. TLS do Caddy cấp (internal CA).
- [x] UI: danh sách Ingress cùng URL truy cập; nút mở trình duyệt.

### 3.3 Quản lý port-forward
- [x] Backend:
  - `kubectl port-forward` chạy nền (svc/pod), quản lý tiến trình;
  - tự kết nối lại khi pod restart;
  - chọn port host trống ngoài dải ephemeral;
  - dừng tất cả khi cluster stop hoặc app thoát.
- [x] Lưu danh sách forward để bật lại khi cluster start (tuỳ chọn).
- [x] UI: nút Forward ở tab Services/Pods; panel "Port-forwards" trên ClusterCard.
- [x] MCP: `portforward_start`, `portforward_stop`, `portforward_list`.

### 3.4 Liên kết service giữa các cluster
- [x] Các cluster cùng network `kind`. Tạo Service ExternalName/Endpoints trỏ tới NodePort hoặc LoadBalancer IP (MetalLB) của cluster kia.
- [x] UI: chọn service nguồn → cluster đích → tên alias. Danh sách các liên kết, kiểm tra kết nối.
- [x] MCP: `cluster_link_service`, `cluster_unlink_service`.

## Giai đoạn 4 — Helm và clone
### 4.1 Helm releases
- [x] Cài binary helm riêng của app (giống kind), ghim phiên bản và kiểm tra checksum.
- [x] Danh sách release (`helm list -A -o json`), xem values/manifest/history, upgrade, rollback, uninstall.
- [x] Repo: thêm/xoá, tìm chart (`helm search repo`), cài chart với values editor (YAML có diff) và namespace.
- [x] UI: tab Helm trong ClusterMonitor.
- [x] MCP: `helm_list`, `helm_install`, `helm_upgrade`, `helm_rollback`, `helm_uninstall` (cần confirm). — upgrade dùng `helm_install` với `upgrade: true`; có thêm `helm_release`, `helm_repo_*`, `helm_search`, `helm_show_values`.

### 4.2 Clone/snapshot cluster
- [x] Snapshot:
  - dừng cluster, `docker commit` từng node thành image `hsx2dock-snap/<cluster>:<tag>`;
  - lưu kind config và metadata;
  - chép state dir containerd của node (dùng chung đường dẫn xoá/chép trong userns).
- [x] Clone: tạo cluster mới từ snapshot (tên mới, port API mới). Xử lý chứng chỉ và hostname: ghi lại hoặc cảnh báo giới hạn. — Clone giữ tên node Kubernetes cũ (`--hostname-override`), container dùng hostname mới và IP tĩnh trên network `kind`, `ExtraHosts` map tên cũ về node của clone. Chỉ clone được cluster 1 control-plane.
- [x] Danh sách snapshot, dung lượng, xoá, khôi phục.
- [x] MCP: `cluster_snapshot`, `cluster_clone`, `snapshot_list`, `snapshot_delete`. — tên thực tế: `cluster_snapshot_create`, `cluster_snapshot_list`, `cluster_snapshot_delete`, `cluster_snapshot_restore`, `cluster_clone`.

## Giai đoạn 5 — Giám sát
### 5.1 Cảnh báo sự cố qua thông báo desktop
- [x] Watcher nền cho cluster đang chạy, theo dõi:
  - pod CrashLoopBackOff/ImagePullBackOff/OOMKilled;
  - node NotReady;
  - API không phản hồi;
  - PVC Pending;
  - restart tăng đột biến.
- [x] Chống spam: gộp sự kiện, cooldown cho mỗi đối tượng, chỉ báo khi trạng thái đổi.
- [x] Cấu hình: bật/tắt theo cluster và theo loại sự cố (tích hợp NotificationsCard hiện có). Bấm vào thông báo thì mở đúng cluster/pod.
- [x] MCP: `cluster_alerts` (danh sách sự cố gần đây).

### 5.2 Biểu đồ lịch sử tài nguyên
- [x] Lưu mẫu CPU, mem, disk, pods theo cluster/node ra file (ring buffer: 24h ở độ phân giải 1 phút, 7 ngày ở 15 phút). — Giới hạn: lưu CPU, mem, số pod theo cluster; chưa lưu disk và chưa tách theo node.
- [x] Tuỳ chọn metrics-server addon để lấy số liệu theo pod/namespace.
- [x] UI: tab History trong ClusterMonitor, chọn khoảng thời gian (1h/6h/24h/7d), chồng nhiều series, tooltip.
- [x] Sparkline trên ClusterCard dùng dữ liệu lịch sử thật (không mất khi reload).


## Kiểm tra Giai đoạn 2–5 (2026-11-25)
Test live qua `hsx2dock-mcp` trên cluster `lab` (1 CP + 1 worker) và bản clone `lab2`:
- Tạo cluster theo spec (worker, ingress, local registry): OK. Thêm và bớt worker: OK. Push vào `localhost:5001` rồi pod pull lại: OK.
- Addon ingress-nginx, MetalLB, metrics-server, cert-manager, dashboard: cài và gỡ OK.
- Ingress qua cổng 18080 và qua Caddy (HTTP/HTTPS): OK.
- Port-forward service → `127.0.0.1:18555`: OK.
- Helm repo/search/install/upgrade/rollback: OK.
- Snapshot, restore tại chỗ, clone, xoá snapshot: OK. Cluster đi qua engine restart, IP tĩnh của clone được giữ.
- Service link lab → lab2 qua LoadBalancer, DNS trong pod: OK.
- GUI: history ghi mỗi phút vào `~/.local/share/hsx2dock/kind-history/`. Thông báo desktop "Cluster lab: 2 new problems" (crash-loop, image-pull) đã bắn.

Lỗi tìm thấy khi test và đã sửa:
- ingress-nginx v1.15.1 không ghim controller vào node `ingress-ready` → patch nodeSelector. Chờ bằng `rollout status` thay vì `wait Available`.
- Host kubeconfig cho port-forward/Helm ghi nhầm đường dẫn thay vì nội dung → dùng `kind get kubeconfig`.
- MetalLB của mọi cluster dùng chung một dải IP → mỗi cluster một khối 16 địa chỉ riêng; gán lại sau restore và clone.
- Docker 29 đăng ký hostname vào DNS của network: clone giữ hostname cũ làm hai cluster lẫn node của nhau → hostname mới, IP tĩnh, `ExtraHosts`, `--hostname-override`. Quét mọi DaemonSet/Deployment còn trỏ endpoint cũ (kindnet).
- Kubernetes 1.37 để container crash-loop ở trạng thái `terminated` giữa các lần back-off → coi là crash-loop khi exit ≠ 0 và restart ≥ 3.
- `cluster_alerts` qua MCP rỗng (watcher chỉ chạy trong GUI) → MCP quét live.
- Xoá snapshot đang được node dùng bị từ chối → chỉ gỡ tag image.
- Registry không tự chạy lại sau engine restart → tự bật khi cluster đã nối registry khởi động.
- Auto-heal "stale endpoint" làm mất IP tĩnh → giữ IPAM config khi nối lại.
- Tên clone ở UI không theo luật backend → `validateClusterName` theo `check_cluster_name`.

Chưa kiểm tra bằng mắt: giao diện các tab mới (Addons, Ingress, Helm, Forwards, History, Alerts, Snapshots). Lý do: trên Wayland, công cụ tự động không chụp và không điều khiển được cửa sổ app.

## Compose — Orchestration & Gateway › Compose (cập nhật 2026-11-25)

Trạng thái: **chưa commit**. Code nằm trong working tree. Bản release đã được cài, app đang chạy bản mới.

### C1 — Đã xong và đã kiểm tra

#### Backend (Rust)
- [x] `docker/compose.rs` viết lại:
  - `Target` (file, project, env_file, profiles) dựng global args: `--ansi never --progress plain`. Tên project và service được kiểm tra trước khi chạy.
  - Up options: force_recreate, remove_orphans, services, profiles, env_file. Down options: remove_volumes, remove_orphans. Down dùng `--profile '*'`.
  - Action theo service: start, stop, restart, up, recreate, scale (tối đa 100).
  - Chạy dạng streaming: mỗi dòng output thành event `compose-progress {op_id, line}`. Timeout: query 60s, thao tác 15 phút.
  - `config --format json` → `ComposeModel` (services, depends_on, ports, profiles, healthcheck, replicas, container_name).
  - Validate YAML chưa lưu qua stdin (`-f -` + `--project-directory`).
  - Quét thư mục tìm file compose (bỏ qua thư mục ẩn, node_modules, target…; giới hạn 200 kết quả).
  - Đọc/ghi `.env`: chặn đường dẫn tuyệt đối và `..`. Ghi có kiểm tra (so với nội dung gốc, ghi file tạm rồi rename, giữ quyền file).
- [x] `commands/compose.rs`: danh sách project gộp 3 nguồn (Settings, project đã ghim, label container). Cache model theo mtime của file compose và `.env`.
- [x] Lệnh mới: pin/unpin, scan, service action, validate, đọc/lưu file compose, đọc/lưu env, mở thư mục.
- [x] Config: `compose_projects` (danh sách ghim). `preserve_owned_fields` giữ trường này khi lưu Settings.
- [x] MCP:
  - `compose_up`, `compose_down` (cần confirm), `compose_logs` (lọc theo service) có thêm tuỳ chọn;
  - tool mới: `compose_service_action`, `compose_config`, `compose_project_pin`, `compose_project_unpin`, `compose_scan`.

#### Frontend
- [x] Thư viện:
  - `lib/composeProject.ts`: gộp service khai báo với container, trạng thái, ports, health, lọc, sắp xếp, layer phụ thuộc, domain mặc định.
  - `lib/dotenv.ts`: parse và serialize giữ nguyên comment.
  - `lib/textDiff.ts`: diff LCS có ngữ cảnh.
  - Test: `lib/compose.test.ts`.
- [x] `ComposePage` viết lại:
  - Header: Add project, Refresh.
  - Toolbar: tìm kiếm, lọc trạng thái kèm số đếm, sắp xếp.
  - Card full-width, empty state, skeleton.
  - Tự làm mới mỗi 5s (dừng khi cửa sổ ẩn). CPU/RAM theo `container-stats-tick`.
- [x] `ComposeProjectCard`:
  - icon trạng thái, badges (lỗi file, health, env, profiles);
  - action: Up/Restart, Stop, Mở chi tiết;
  - menu ⋯: Up có tuỳ chọn, Ghim/Bỏ ghim, Copy path, Mở thư mục, Down;
  - info strip: Services, Containers, Ports, CPU, Memory, Source.
- [x] `ComposeDrawer`, 6 tab:
  - **Services:** action theo từng service, replicas, ports, health, CPU/RAM, domain Gateway.
  - **Logs:** realtime, lọc theo service.
  - **Graph:** đồ thị `depends_on`.
  - **Compose file:** editor, Validate, Diff, Revert, Save, Save & apply. Ctrl+S để lưu.
  - **Env:** bảng hoặc text thô, ẩn giá trị bí mật, cảnh báo key trùng hoặc sai tên.
  - **Terminal:** shell vào container đang chạy.
  - Đóng drawer khi chưa lưu sẽ hỏi xác nhận. Editor vẫn mounted khi chuyển tab.
- [x] Dialogs:
  - Add project (nhập path hoặc quét thư mục);
  - Up options (profiles, env file, force recreate, remove orphans);
  - Down (remove orphans, remove volumes);
  - Scale;
  - Expose qua Gateway (chọn port, domain, path; lưu route; mở URL).
- [x] `ComposeProgressPanel`: panel thao tác góc dưới phải, có spinner, output trực tiếp, thời gian chạy, lỗi; dọn được thao tác đã xong.
- [x] `GroupLogsModal` có prop `embedded` để nhúng vào drawer.

#### Kiểm tra
- [x] Rust: 366 tests pass, clippy sạch. Frontend: 185 tests pass, `tsc` sạch.
- [x] Test live qua MCP trên project `cdemo` (web → api → db, `debug` thuộc profile `tools`, có `.env`):
  - scan, validate (YAML đúng và sai), pin, up có profile và env_file;
  - restart, scale db=2 rồi về 1, up service có profile;
  - logs lọc theo service;
  - scale thiếu replicas báo lỗi đúng; `compose_down` bắt buộc confirm.

### C2 — Việc còn lại (ưu tiên cao)
- [ ] **Kiểm tra UI bằng mắt.** Trên Wayland, công cụ tự động không chụp và không điều khiển được cửa sổ app. Cần mở trang Compose và đi hết:
  - card;
  - toolbar (tìm kiếm, lọc, sắp xếp);
  - 6 tab trong drawer;
  - các dialog;
  - panel tiến trình;
  - Expose rồi mở domain qua Gateway.
- [ ] Commit khi đã xem xong UI.
- [x] Restart/Stop/Down theo project có `op_id` và stream output vào panel tiến trình (`compose::project_action_streaming`). Nhóm container và MCP vẫn dùng bản không stream.
- [x] ScaleDialog cảnh báo khi số replicas khác file: lần Up thường sau sẽ đưa về giá trị trong file. Muốn giữ thì sửa `deploy.replicas` trong tab Compose file.
- [x] Validate tìm ra dòng lỗi (`lib/yamlLocate.ts`, có test) từ ba dạng lỗi: `yaml: line N`, key path `services.web.portz` và `service "web" depends on…`. Editor tô đỏ số dòng lỗi và có nút "Go to line N".
- [x] Tab Env: mỗi dòng có id ổn định (`uid`), nên xoá hoặc thêm dòng không làm lệch trạng thái ẩn/hiện. Revert ẩn lại mọi giá trị.
- [x] Drawer và lựa chọn project dùng key `name|path`. Trạng thái busy vẫn theo tên project, vì cùng tên nghĩa là cùng một bộ container.

### C3 — Tính năng mở rộng (đề xuất, chưa chọn)
#### C3.1 Pull và build (lần trước chưa chọn)
- [ ] `compose pull` theo project hoặc theo service, có tiến trình trong panel.
- [ ] `compose build` (có tuỳ chọn `--no-cache` và `--pull`), stream log build.
- [ ] Badge "image có bản mới" khi digest local khác registry.

#### C3.2 Tạo project mới
- [ ] Wizard tạo `compose.yaml` từ template (web + db, wordpress, postgres + pgadmin…). Chọn thư mục, ghi file, ghim, Up.
- [ ] Chuyển một nhóm container đang chạy sang compose: sinh YAML từ inspect.

#### C3.3 Nhiều file và override
- [ ] Hỗ trợ nhiều `-f` (`compose.yaml` + `compose.override.yaml`), chọn file khi Up. Editor có tab cho từng file.
- [ ] Xem config đã merge (`compose config`) ở chế độ chỉ đọc.

#### C3.4 Theo dõi sâu hơn
- [ ] Thay polling 5s bằng event stream của Docker (container start/stop/die/health_status) theo label `com.docker.compose.project`.
- [ ] Lịch sử CPU/RAM theo project (ring buffer như kind-history), sparkline trên card.
- [ ] Thông báo desktop khi service unhealthy, exit ≠ 0 hoặc restart liên tục (dùng lại cơ chế cảnh báo của Giai đoạn 5).
- [ ] Chi tiết healthcheck: lần chạy gần nhất, output, số lần fail.

#### C3.5 Editor
- [ ] Tô màu cú pháp YAML (CodeMirror 6 hoặc highlight nhẹ), gập khối, tìm kiếm.
- [ ] Gợi ý key compose theo JSON schema chính thức.
- [ ] Tự reload khi file bị sửa bên ngoài (file watcher). Cảnh báo xung đột nếu đang có bản nháp.

#### C3.6 Khác
- [ ] `compose watch` (dev mode, sync file) bật/tắt theo project.
- [ ] Xuất và nhập project (YAML, `.env` tuỳ chọn, ẩn secret).
- [ ] Volume của project: dung lượng, backup/restore volume trước khi `down -v`.
- [ ] MCP: `compose_read_file`, `compose_save_file` (cần confirm), `compose_env_get`, `compose_env_set`.


## Ghi chú khác
- Node kind dừng với exit 137 khi tắt engine. Nên xem lại tín hiệu dừng (SIGRTMIN+3) và grace period.
- Follow-up: container publish port ngẫu nhiên hoặc ephemeral không tới được từ host (giới hạn của pasta).
- Repo chưa sạch `cargo fmt` từ trước (compose.rs, kind.rs…). Không format lại trong đợt này.
- Kubernetes Giai đoạn 1–5 đã xong (xem phần kiểm tra ở trên). Việc Compose còn lại nằm ở mục C2 và C3.
