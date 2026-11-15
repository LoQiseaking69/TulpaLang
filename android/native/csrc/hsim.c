// Thin C shim over MuJoCo for the Tulpa Field native bridge. Keeps MuJoCo's struct
// layouts out of Rust: everything is exposed as counts, typed array pointers and a few
// operations. All functions expect a valid HSim* from hs_load().
#include <math.h>
#include <stdlib.h>
#include <string.h>
#include <mujoco/mujoco.h>

typedef struct { mjModel* m; mjData* d; } HSim;

HSim* hs_load(const char* path, char* err, int errsz) {
  mjModel* m = mj_loadXML(path, NULL, err, errsz);
  if (!m) return NULL;
  mjData* d = mj_makeData(m);
  if (!d) { mj_deleteModel(m); strncpy(err, "out of memory", errsz); return NULL; }
  HSim* s = (HSim*)calloc(1, sizeof(HSim));
  s->m = m; s->d = d;
  if (m->nkey > 0) mj_resetDataKeyframe(m, d, 0);
  mj_forward(m, d);
  return s;
}

void hs_free(HSim* s) { if (!s) return; mj_deleteData(s->d); mj_deleteModel(s->m); free(s); }
const char* hs_version(void) { return mj_versionString(); }

enum { C_NQ, C_NV, C_NU, C_NBODY, C_NJNT, C_NSENSOR, C_NKEY, C_NGEOM, C_NMESH, C_NCON, C_NSENSORDATA, C_NMAT };
long hs_count(HSim* s, int what) {
  mjModel* m = s->m;
  switch (what) {
    case C_NQ: return m->nq; case C_NV: return m->nv; case C_NU: return m->nu;
    case C_NBODY: return m->nbody; case C_NJNT: return m->njnt; case C_NSENSOR: return m->nsensor;
    case C_NKEY: return m->nkey; case C_NGEOM: return m->ngeom; case C_NMESH: return m->nmesh;
    case C_NCON: return s->d->ncon; case C_NSENSORDATA: return m->nsensordata; case C_NMAT: return m->nmat;
  }
  return 0;
}

// object types follow mjtObj
enum { O_BODY = 1, O_JOINT = 3, O_GEOM = 5, O_ACTUATOR = 19, O_SENSOR = 22, O_KEY = 23 };
const char* hs_name(HSim* s, int type, int id) {
  int t = type == O_BODY ? mjOBJ_BODY : type == O_JOINT ? mjOBJ_JOINT : type == O_GEOM ? mjOBJ_GEOM
        : type == O_ACTUATOR ? mjOBJ_ACTUATOR : type == O_SENSOR ? mjOBJ_SENSOR : mjOBJ_KEY;
  return mj_id2name(s->m, t, id);
}
int hs_id(HSim* s, int type, const char* name) {
  int t = type == O_BODY ? mjOBJ_BODY : type == O_JOINT ? mjOBJ_JOINT : type == O_KEY ? mjOBJ_KEY : mjOBJ_ACTUATOR;
  return mj_name2id(s->m, t, name);
}

// mjtNum (double) arrays
enum { D_QPOS, D_QVEL, D_CTRL, D_ACTFORCE, D_SENSORDATA, D_XPOS, D_XQUAT, D_SUBCOM, D_SUBLINVEL,
       D_CTRLRANGE, D_JNTRANGE, D_XFRC, D_GEOMXPOS, D_GEOMXMAT, D_GEOMSIZE, D_BODYMASS };
double* hs_dptr(HSim* s, int f) {
  mjModel* m = s->m; mjData* d = s->d;
  switch (f) {
    case D_QPOS: return d->qpos; case D_QVEL: return d->qvel; case D_CTRL: return d->ctrl;
    case D_ACTFORCE: return d->actuator_force; case D_SENSORDATA: return d->sensordata;
    case D_XPOS: return d->xpos; case D_XQUAT: return d->xquat; case D_SUBCOM: return d->subtree_com;
    case D_SUBLINVEL: return d->subtree_linvel; case D_CTRLRANGE: return m->actuator_ctrlrange;
    case D_JNTRANGE: return m->jnt_range; case D_XFRC: return d->xfrc_applied;
    case D_GEOMXPOS: return d->geom_xpos; case D_GEOMXMAT: return d->geom_xmat;
    case D_GEOMSIZE: return m->geom_size; case D_BODYMASS: return m->body_mass;
  }
  return NULL;
}

// int arrays
enum { I_TRNID, I_JNTTYPE, I_JNTQPOSADR, I_JNTDOFADR, I_SENSTYPE, I_SENSDIM, I_SENSADR,
       I_GEOMTYPE, I_GEOMDATAID, I_GEOMBODY, I_GEOMMAT, I_GEOMGROUP };
int* hs_iptr(HSim* s, int f) {
  mjModel* m = s->m;
  switch (f) {
    case I_TRNID: return m->actuator_trnid; case I_JNTTYPE: return m->jnt_type;
    case I_JNTQPOSADR: return m->jnt_qposadr; case I_JNTDOFADR: return m->jnt_dofadr;
    case I_SENSTYPE: return m->sensor_type; case I_SENSDIM: return m->sensor_dim; case I_SENSADR: return m->sensor_adr;
    case I_GEOMTYPE: return m->geom_type; case I_GEOMDATAID: return m->geom_dataid;
    case I_GEOMBODY: return m->geom_bodyid; case I_GEOMMAT: return m->geom_matid; case I_GEOMGROUP: return m->geom_group;
  }
  return NULL;
}

int hs_ctrllimited(HSim* s, int i) { return s->m->actuator_ctrllimited[i] ? 1 : 0; }
double hs_gear(HSim* s, int i) { return s->m->actuator_gear[6 * i]; }
double hs_timestep(HSim* s) { return s->m->opt.timestep; }
void hs_gravity(HSim* s, double* out) { for (int k = 0; k < 3; k++) out[k] = s->m->opt.gravity[k]; }
double hs_time(HSim* s) { return s->d->time; }
void hs_stat(HSim* s, double* out) { for (int k = 0; k < 3; k++) out[k] = s->m->stat.center[k]; out[3] = s->m->stat.extent; }
int hs_energy(HSim* s, double* out) {
  if (!(s->m->opt.enableflags & mjENBL_ENERGY)) return 0;
  out[0] = s->d->energy[0]; out[1] = s->d->energy[1]; return 1;
}

void hs_step(HSim* s) { mj_step(s->m, s->d); }
void hs_forward(HSim* s) { mj_forward(s->m, s->d); }
void hs_subtree_vel(HSim* s) { mj_subtreeVel(s->m, s->d); }
void hs_reset(HSim* s, int key) {
  if (key >= 0 && key < s->m->nkey) mj_resetDataKeyframe(s->m, s->d, key); else mj_resetData(s->m, s->d);
  mju_zero(s->d->xfrc_applied, 6 * s->m->nbody);
  mj_forward(s->m, s->d);
}

// contact i: bodies, normal force magnitude, position
void hs_contact(HSim* s, int i, int* b1, int* b2, double* fn, double* pos) {
  mjContact* c = s->d->contact + i;
  *b1 = s->m->geom_bodyid[c->geom1]; *b2 = s->m->geom_bodyid[c->geom2];
  mjtNum f6[6]; mj_contactForce(s->m, s->d, i, f6);
  *fn = fabs(f6[0]);
  for (int k = 0; k < 3; k++) pos[k] = c->pos[k];
}

// geom colour: material rgba when a material is set, otherwise geom rgba
void hs_geom_rgba(HSim* s, int g, float* out) {
  int mat = s->m->geom_matid[g];
  const float* src = mat >= 0 ? s->m->mat_rgba + 4 * mat : s->m->geom_rgba + 4 * g;
  for (int k = 0; k < 4; k++) out[k] = src[k];
}

// Mesh as a flat triangle list, 6 floats per vertex (position, normal). Call with out=NULL
// to get the number of floats needed.
long hs_mesh_tris(HSim* s, int mesh, float* out) {
  mjModel* m = s->m;
  int nf = m->mesh_facenum[mesh];
  if (!out) return (long)nf * 18;
  const float* v = m->mesh_vert + 3 * (long)m->mesh_vertadr[mesh];
  const int* f = m->mesh_face + 3 * (long)m->mesh_faceadr[mesh];
  const float* nrm = m->mesh_normal ? m->mesh_normal + 3 * (long)m->mesh_normaladr[mesh] : NULL;
  const int* fn = m->mesh_facenormal ? m->mesh_facenormal + 3 * (long)m->mesh_faceadr[mesh] : NULL;
  long o = 0;
  for (int i = 0; i < nf; i++) {
    const float* a = v + 3 * f[3 * i]; const float* b = v + 3 * f[3 * i + 1]; const float* c = v + 3 * f[3 * i + 2];
    float ux = b[0] - a[0], uy = b[1] - a[1], uz = b[2] - a[2];
    float wx = c[0] - a[0], wy = c[1] - a[1], wz = c[2] - a[2];
    float nx = uy * wz - uz * wy, ny = uz * wx - ux * wz, nz = ux * wy - uy * wx;
    float l = sqrtf(nx * nx + ny * ny + nz * nz); if (l > 0) { nx /= l; ny /= l; nz /= l; }
    for (int k = 0; k < 3; k++) {
      const float* p = v + 3 * f[3 * i + k];
      out[o++] = p[0]; out[o++] = p[1]; out[o++] = p[2];
      if (nrm && fn) { const float* q = nrm + 3 * fn[3 * i + k]; out[o++] = q[0]; out[o++] = q[1]; out[o++] = q[2]; }
      else { out[o++] = nx; out[o++] = ny; out[o++] = nz; }
    }
  }
  return o;
}
